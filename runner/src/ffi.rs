use std::os::raw::c_char;

extern "C" {
    pub fn bocfel_run_interactive(story_path: *const c_char) -> i32;
}
