use crate::agent::{AgentEvent, AgentTask, run_single_turn};
use crate::config::AppConfig;
use crate::game::{GameSession, Transcript};
use crate::llm::LlmClient;
use crate::logging::SessionLogger;
use crate::memory::WorldModel;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use std::io;
use std::time::{Duration, Instant};

fn append_history_cell(history_lines: &mut Vec<String>, title: &str, body: &str, width: usize) {
    history_lines.extend(render_cell_lines(title, body, width));
}

fn append_agent_cell(
    history_lines: &mut Vec<String>,
    thought: Option<&str>,
    action: Option<&str>,
    width: usize,
) {
    let mut sections = Vec::new();
    if let Some(thought) = thought {
        sections.push(format!("Thought:\n{thought}"));
    }
    if let Some(action) = action {
        sections.push(format!("Action:\n{action}"));
    }

    append_history_cell(history_lines, "AGENT", &sections.join("\n\n"), width);
}

fn render_cell_lines(title: &str, body: &str, width: usize) -> Vec<String> {
    let width = width.max(2);
    let content_width = width.saturating_sub(2);
    let mut lines = Vec::new();
    lines.push(render_cell_top(title, width));

    for line in body.lines() {
        for wrapped in wrap_line(line, content_width) {
            lines.push(format!("│{wrapped:<content_width$}│"));
        }
    }

    if body.ends_with('\n') {
        lines.push(format!("│{:<content_width$}│", ""));
    }

    lines.push(format!("└{}┘", "─".repeat(content_width)));
    lines
}

fn render_cell_top(title: &str, width: usize) -> String {
    let content_width = width.saturating_sub(2);
    let title = format!("─ {title} ");
    let title_len = title.chars().count();

    if title_len >= content_width {
        return format!("┌{}┐", trim_to_width(&title, content_width));
    }

    format!("┌{}{}┐", title, "─".repeat(content_width - title_len))
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    if line.is_empty() {
        return vec![String::new()];
    }

    let mut remaining = line.trim_end();
    let mut wrapped = Vec::new();
    while remaining.chars().count() > width {
        let mut split_at = byte_index_at_width(remaining, width);
        let prefix = &remaining[..split_at];
        if let Some((space_index, _)) = prefix
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
        {
            if space_index > 0 {
                split_at = space_index;
            }
        }

        let segment = remaining[..split_at].trim_end();
        wrapped.push(segment.to_string());
        remaining = remaining[split_at..].trim_start();
    }

    wrapped.push(remaining.to_string());
    wrapped
}

fn byte_index_at_width(value: &str, width: usize) -> usize {
    value
        .char_indices()
        .nth(width)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}

fn trim_to_width(value: &str, width: usize) -> String {
    value.chars().take(width).collect()
}

fn visible_history_bounds(
    total_lines: usize,
    viewport_height: usize,
    requested_start: Option<usize>,
) -> (usize, usize) {
    if total_lines == 0 || viewport_height == 0 {
        return (0, 0);
    }

    let visible_height = viewport_height.min(total_lines);
    let max_start = total_lines.saturating_sub(visible_height);
    let start = requested_start.unwrap_or(max_start).min(max_start);
    let end = (start + visible_height).min(total_lines);
    (start, end)
}

fn scroll_history_up(history_start: &mut Option<usize>, visible_start: usize, amount: usize) {
    *history_start = Some(visible_start.saturating_sub(amount.max(1)));
}

fn scroll_history_down(
    history_start: &mut Option<usize>,
    visible_start: usize,
    total_lines: usize,
    viewport_height: usize,
    amount: usize,
) {
    let next_start = visible_start.saturating_add(amount.max(1));
    let (_, bottom_end) = visible_history_bounds(total_lines, viewport_height, None);
    if next_start + viewport_height >= bottom_end {
        *history_start = None;
    } else {
        *history_start = Some(next_start);
    }
}

fn trim_history_lines(
    history_lines: &mut Vec<String>,
    max_history_lines: usize,
    history_start: &mut Option<usize>,
) {
    if max_history_lines == 0 {
        history_lines.clear();
        *history_start = None;
        return;
    }

    if history_lines.len() > max_history_lines {
        let drop_count = history_lines.len().saturating_sub(max_history_lines);
        history_lines.drain(0..drop_count);
        if let Some(start) = history_start {
            *start = start.saturating_sub(drop_count);
        }
    }
}

pub fn run_tui(
    config: AppConfig,
    mut game: GameSession,
    mut world: WorldModel,
    llm: LlmClient,
    logger: SessionLogger,
) -> anyhow::Result<(Transcript, WorldModel)> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = ui_loop(&mut terminal, &config, &mut game, &mut world, &llm, &logger);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;

    result.map(|_| (game.transcript().clone(), world))
}

fn ui_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    config: &AppConfig,
    game: &mut GameSession,
    world: &mut WorldModel,
    llm: &LlmClient,
    logger: &SessionLogger,
) -> anyhow::Result<()> {
    let mut history_lines = Vec::<String>::new();
    let mut history_start: Option<usize> = None;
    let mut history_area_width = terminal.size()?.width.saturating_sub(1) as usize;
    let mut history_area_height = 0usize;
    let mut visible_history_start = 0usize;
    let mut input = String::new();
    let mut input_history = Vec::<String>::new();
    let mut history_cursor: Option<usize> = None;
    let mut active_task: Option<AgentTask> = None;
    let mut pending_agent_thought: Option<String> = None;
    let mut last_agent_tick = Instant::now();

    if let Ok(obs) = game.execute("look") {
        append_history_cell(
            &mut history_lines,
            "GAME OUTPUT",
            &obs.text,
            history_area_width,
        );
    }

    loop {
        if let Some(task) = &mut active_task {
            if last_agent_tick.elapsed() >= Duration::from_millis(150) {
                for ev in run_single_turn(task, game, world, llm, logger) {
                    match ev {
                        AgentEvent::Thought(t) => {
                            if let Some(previous) = pending_agent_thought.replace(t) {
                                append_agent_cell(
                                    &mut history_lines,
                                    Some(&previous),
                                    None,
                                    history_area_width,
                                );
                            }
                        }
                        AgentEvent::Action(a) => {
                            let thought = pending_agent_thought.take();
                            append_agent_cell(
                                &mut history_lines,
                                thought.as_deref(),
                                Some(&a),
                                history_area_width,
                            );
                        }
                        AgentEvent::Observation(o) => {
                            if let Some(thought) = pending_agent_thought.take() {
                                append_agent_cell(
                                    &mut history_lines,
                                    Some(&thought),
                                    None,
                                    history_area_width,
                                );
                            }
                            append_history_cell(
                                &mut history_lines,
                                "GAME OUTPUT",
                                &o,
                                history_area_width,
                            );
                        }
                        AgentEvent::Completed(s) => {
                            if let Some(thought) = pending_agent_thought.take() {
                                append_agent_cell(
                                    &mut history_lines,
                                    Some(&thought),
                                    None,
                                    history_area_width,
                                );
                            }
                            append_history_cell(
                                &mut history_lines,
                                "SYSTEM MESSAGE",
                                &s,
                                history_area_width,
                            );
                            active_task = None;
                        }
                        AgentEvent::Failed(s) => {
                            if let Some(thought) = pending_agent_thought.take() {
                                append_agent_cell(
                                    &mut history_lines,
                                    Some(&thought),
                                    None,
                                    history_area_width,
                                );
                            }
                            append_history_cell(
                                &mut history_lines,
                                "SYSTEM MESSAGE",
                                &s,
                                history_area_width,
                            );
                            active_task = None;
                        }
                    }
                }
                last_agent_tick = Instant::now();
            }
        }

        trim_history_lines(
            &mut history_lines,
            config.ui.max_history_lines,
            &mut history_start,
        );

        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(4), Constraint::Length(3)])
                .split(frame.size());

            let history_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1), Constraint::Length(1)])
                .split(chunks[0]);
            let history_area = history_chunks[0];
            let scrollbar_area = history_chunks[1];

            history_area_width = history_area.width as usize;
            history_area_height = history_area.height as usize;
            let (start, end) =
                visible_history_bounds(history_lines.len(), history_area_height, history_start);
            visible_history_start = start;
            let history = history_lines[start..end].join("\n");

            let history_widget = Paragraph::new(history).style(Style::default().fg(Color::White));

            let input_widget = Paragraph::new(format!("> {input}")).block(
                Block::default()
                    .title("INPUT")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Magenta)),
            );

            frame.render_widget(history_widget, history_area);
            if history_lines.len() > history_area_height {
                let scroll_positions = history_lines
                    .len()
                    .saturating_sub(history_area_height)
                    .saturating_add(1);
                let mut scrollbar_state = ScrollbarState::new(scroll_positions)
                    .viewport_content_length(history_area_height)
                    .position(visible_history_start);
                frame.render_stateful_widget(
                    Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight),
                    scrollbar_area,
                    &mut scrollbar_state,
                );
            }
            frame.render_widget(input_widget, chunks[1]);
            let cursor_x = (chunks[1].x + 3 + input.chars().count() as u16)
                .min(chunks[1].x + chunks[1].width.saturating_sub(2));
            frame.set_cursor(cursor_x, chunks[1].y + 1);
        })?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if handle_input(
                                "exit".to_string(),
                                game,
                                world,
                                logger,
                                &mut history_lines,
                                history_area_width,
                                &mut active_task,
                            )? {
                                break;
                            }
                        }
                        KeyCode::Char(c) => {
                            history_cursor = None;
                            input.push(c);
                        }
                        KeyCode::Backspace => {
                            history_cursor = None;
                            input.pop();
                        }
                        KeyCode::Up => {
                            if !input_history.is_empty() {
                                let index = match history_cursor {
                                    Some(0) => 0,
                                    Some(index) => index - 1,
                                    None => input_history.len() - 1,
                                };
                                history_cursor = Some(index);
                                input = input_history[index].clone();
                            }
                        }
                        KeyCode::Down => {
                            if let Some(index) = history_cursor {
                                if index + 1 < input_history.len() {
                                    let next_index = index + 1;
                                    history_cursor = Some(next_index);
                                    input = input_history[next_index].clone();
                                } else {
                                    history_cursor = None;
                                    input.clear();
                                }
                            }
                        }
                        KeyCode::PageUp => {
                            scroll_history_up(
                                &mut history_start,
                                visible_history_start,
                                history_area_height,
                            );
                        }
                        KeyCode::PageDown => {
                            scroll_history_down(
                                &mut history_start,
                                visible_history_start,
                                history_lines.len(),
                                history_area_height,
                                history_area_height,
                            );
                        }
                        KeyCode::Home => {
                            history_start = Some(0);
                        }
                        KeyCode::End => {
                            history_start = None;
                        }
                        KeyCode::Enter => {
                            let line = input.trim().to_string();
                            input.clear();
                            history_cursor = None;
                            record_input_history(
                                &mut input_history,
                                config.ui.input_history_limit,
                                &line,
                            );
                            if handle_input(
                                line,
                                game,
                                world,
                                logger,
                                &mut history_lines,
                                history_area_width,
                                &mut active_task,
                            )? {
                                break;
                            }
                        }
                        KeyCode::Esc => {
                            break;
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        scroll_history_up(&mut history_start, visible_history_start, 3);
                    }
                    MouseEventKind::ScrollDown => {
                        scroll_history_down(
                            &mut history_start,
                            visible_history_start,
                            history_lines.len(),
                            history_area_height,
                            3,
                        );
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    Ok(())
}

fn record_input_history(history: &mut Vec<String>, limit: usize, line: &str) {
    if limit == 0 || line.is_empty() || history.last().is_some_and(|last| last == line) {
        return;
    }

    history.push(line.to_string());
    if history.len() > limit {
        let drop_count = history.len().saturating_sub(limit);
        history.drain(0..drop_count);
    }
}

fn handle_input(
    line: String,
    game: &mut GameSession,
    world: &mut WorldModel,
    logger: &SessionLogger,
    history_lines: &mut Vec<String>,
    history_width: usize,
    active_task: &mut Option<AgentTask>,
) -> anyhow::Result<bool> {
    if line.is_empty() {
        return Ok(false);
    }

    logger.log("user_input", &line);

    if line == "exit" || line == "/exit" {
        append_history_cell(
            history_lines,
            "SYSTEM MESSAGE",
            "Exiting session.",
            history_width,
        );
        return Ok(true);
    }

    if line == "/cancel" {
        if active_task.is_some() {
            *active_task = None;
            append_history_cell(
                history_lines,
                "SYSTEM MESSAGE",
                "Task canceled.",
                history_width,
            );
        } else {
            append_history_cell(
                history_lines,
                "SYSTEM MESSAGE",
                "No active task.",
                history_width,
            );
        }
        return Ok(false);
    }

    if let Some(cmd) = line.strip_prefix("/send") {
        let cmd = cmd.trim();
        append_history_cell(history_lines, "USER COMMAND", cmd, history_width);
        let obs = match game.execute(cmd) {
            Ok(obs) => obs.text,
            Err(e) => format!("Failed to run command: {e}"),
        };
        world.update_from_observation_with_command(&obs, Some(cmd));
        append_history_cell(history_lines, "GAME OUTPUT", &obs, history_width);
        return Ok(false);
    }

    append_history_cell(history_lines, "USER TASK", &line, history_width);
    if active_task.is_some() {
        append_history_cell(
            history_lines,
            "SYSTEM MESSAGE",
            "A task is already running. Use /cancel first.",
            history_width,
        );
        return Ok(false);
    }

    *active_task = Some(AgentTask::new(line));
    append_history_cell(
        history_lines,
        "SYSTEM MESSAGE",
        "Task running... type /cancel to interrupt.",
        history_width,
    );
    Ok(false)
}
