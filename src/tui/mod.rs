use crate::agent::{run_single_turn, AgentEvent, AgentTask};
use crate::config::AppConfig;
use crate::game::{GameSession, Transcript};
use crate::llm::LlmClient;
use crate::logging::SessionLogger;
use crate::memory::WorldModel;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use std::io;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct Cell {
    title: String,
    body: String,
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
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = ui_loop(&mut terminal, &config, &mut game, &mut world, &llm, &logger);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
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
    let mut cells = Vec::<Cell>::new();
    let mut input = String::new();
    let mut active_task: Option<AgentTask> = None;
    let mut last_agent_tick = Instant::now();

    if let Ok(obs) = game.execute("look") {
        cells.push(Cell {
            title: "GAME OUTPUT".to_string(),
            body: obs.text,
        });
    }

    loop {
        if let Some(task) = &mut active_task {
            if last_agent_tick.elapsed() >= Duration::from_millis(150) {
                for ev in run_single_turn(task, game, world, llm, logger) {
                    match ev {
                        AgentEvent::Thought(t) => cells.push(Cell {
                            title: "AGENT".to_string(),
                            body: format!("Thought:\n{t}"),
                        }),
                        AgentEvent::Action(a) => cells.push(Cell {
                            title: "AGENT".to_string(),
                            body: format!("Action:\n{a}"),
                        }),
                        AgentEvent::Observation(o) => cells.push(Cell {
                            title: "GAME OUTPUT".to_string(),
                            body: o,
                        }),
                        AgentEvent::Completed(s) => {
                            cells.push(Cell {
                                title: "SYSTEM MESSAGE".to_string(),
                                body: s,
                            });
                            active_task = None;
                        }
                        AgentEvent::Failed(s) => {
                            cells.push(Cell {
                                title: "SYSTEM MESSAGE".to_string(),
                                body: s,
                            });
                            active_task = None;
                        }
                    }
                }
                last_agent_tick = Instant::now();
            }
        }

        if cells.len() > config.ui.max_visible_cells {
            let drop_count = cells.len().saturating_sub(config.ui.max_visible_cells);
            cells.drain(0..drop_count);
        }

        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(4), Constraint::Length(3)])
                .split(frame.size());

            let history = cells
                .iter()
                .map(|c| format!("┌─ {} ─\n{}\n└────────\n", c.title, c.body))
                .collect::<Vec<_>>()
                .join("\n");

            let history_widget = Paragraph::new(history)
                .block(Block::default().title("Cells").borders(Borders::ALL))
                .wrap(Wrap { trim: false })
                .style(Style::default().fg(Color::White));

            let input_widget = Paragraph::new(format!("> {input}")).block(
                Block::default()
                    .title("INPUT")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Magenta)),
            );

            frame.render_widget(history_widget, chunks[0]);
            frame.render_widget(input_widget, chunks[1]);
        })?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
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
                            &mut cells,
                            &mut active_task,
                        )? {
                            break;
                        }
                    }
                    KeyCode::Char(c) => input.push(c),
                    KeyCode::Backspace => {
                        input.pop();
                    }
                    KeyCode::Enter => {
                        let line = input.trim().to_string();
                        input.clear();
                        if handle_input(line, game, world, logger, &mut cells, &mut active_task)? {
                            break;
                        }
                    }
                    KeyCode::Esc => {
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

fn handle_input(
    line: String,
    game: &mut GameSession,
    world: &mut WorldModel,
    logger: &SessionLogger,
    cells: &mut Vec<Cell>,
    active_task: &mut Option<AgentTask>,
) -> anyhow::Result<bool> {
    if line.is_empty() {
        return Ok(false);
    }

    logger.log("user_input", &line);

    if line == "exit" || line == "/exit" {
        cells.push(Cell {
            title: "SYSTEM MESSAGE".to_string(),
            body: "Exiting session.".to_string(),
        });
        return Ok(true);
    }

    if line == "/cancel" {
        if active_task.is_some() {
            *active_task = None;
            cells.push(Cell {
                title: "SYSTEM MESSAGE".to_string(),
                body: "Task canceled.".to_string(),
            });
        } else {
            cells.push(Cell {
                title: "SYSTEM MESSAGE".to_string(),
                body: "No active task.".to_string(),
            });
        }
        return Ok(false);
    }

    if let Some(cmd) = line.strip_prefix("/send") {
        let cmd = cmd.trim();
        cells.push(Cell {
            title: "USER COMMAND".to_string(),
            body: cmd.to_string(),
        });
        let obs = match game.execute(cmd) {
            Ok(obs) => obs.text,
            Err(e) => format!("Failed to run command: {e}"),
        };
        world.update_from_observation(&obs);
        cells.push(Cell {
            title: "GAME OUTPUT".to_string(),
            body: obs,
        });
        return Ok(false);
    }

    cells.push(Cell {
        title: "USER TASK".to_string(),
        body: line.clone(),
    });
    if active_task.is_some() {
        cells.push(Cell {
            title: "SYSTEM MESSAGE".to_string(),
            body: "A task is already running. Use /cancel first.".to_string(),
        });
        return Ok(false);
    }

    *active_task = Some(AgentTask::new(line));
    cells.push(Cell {
        title: "SYSTEM MESSAGE".to_string(),
        body: "Task running... type /cancel to interrupt.".to_string(),
    });
    Ok(false)
}
