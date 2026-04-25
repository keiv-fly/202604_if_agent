use std::os::raw::{c_char, c_uint};

#[repr(C)]
pub struct BocfelHandle {
    _private: [u8; 0],
}

extern "C" {
    pub fn bocfel_create(story_path: *const c_char) -> *mut BocfelHandle;

    pub fn bocfel_destroy(handle: *mut BocfelHandle);

    pub fn bocfel_send_command(
        handle: *mut BocfelHandle,
        command: *const c_char,
        output_buffer: *mut c_char,
        output_buffer_len: c_uint,
    ) -> i32;

    pub fn bocfel_last_error(handle: *mut BocfelHandle) -> *const c_char;
}
