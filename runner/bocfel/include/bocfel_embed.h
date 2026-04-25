#pragma once

#ifdef __cplusplus
extern "C" {
#endif

typedef struct BocfelHandle BocfelHandle;

BocfelHandle* bocfel_create(const char* story_path);

void bocfel_destroy(BocfelHandle* handle);

int bocfel_send_command(
    BocfelHandle* handle,
    const char* command,
    char* output_buffer,
    unsigned int output_buffer_len
);

int bocfel_run_script(
    BocfelHandle* handle,
    const char* commands,
    char* output_buffer,
    unsigned int output_buffer_len
);

const char* bocfel_last_error(BocfelHandle* handle);

#ifdef __cplusplus
}
#endif
