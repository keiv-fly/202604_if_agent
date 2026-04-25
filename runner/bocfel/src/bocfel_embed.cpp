#include "bocfel_embed.h"

#include <cstdio>
#include <cstring>
#include <fstream>
#include <new>
#include <string>

#ifdef _WIN32
#include <corecrt_io.h>
#else
#include <unistd.h>
#endif

int bocfel_cli_main(int argc, char** argv);

struct BocfelHandle {
    std::string story_path;
    std::string last_error;
};

namespace {

void set_error(BocfelHandle* handle, const std::string& message) {
    if (handle != nullptr) {
        handle->last_error = message;
    }
}

bool file_exists(const char* path) {
    std::ifstream file(path, std::ios::binary);
    return file.good();
}

int duplicate_fd(int fd) {
#ifdef _WIN32
    return _dup(fd);
#else
    return dup(fd);
#endif
}

int duplicate_to(int source_fd, int target_fd) {
#ifdef _WIN32
    return _dup2(source_fd, target_fd);
#else
    return dup2(source_fd, target_fd);
#endif
}

int file_no(FILE* file) {
#ifdef _WIN32
    return _fileno(file);
#else
    return fileno(file);
#endif
}

void close_fd(int fd) {
#ifdef _WIN32
    _close(fd);
#else
    close(fd);
#endif
}

bool write_all(FILE* file, const char* data) {
    return std::fputs(data, file) >= 0 && std::fflush(file) == 0;
}

std::string read_all(FILE* file) {
    std::string result;
    std::rewind(file);

    char buffer[4096];
    while (true) {
        const size_t bytes_read = std::fread(buffer, 1, sizeof(buffer), file);
        result.append(buffer, bytes_read);

        if (bytes_read < sizeof(buffer)) {
            break;
        }
    }

    return result;
}

int copy_output(BocfelHandle* handle, const std::string& output, char* output_buffer, unsigned int output_buffer_len) {
    if (output.size() + 1 > output_buffer_len) {
        set_error(handle, "output buffer is too small");
        return 1;
    }

    std::memcpy(output_buffer, output.c_str(), output.size() + 1);
    handle->last_error.clear();
    return 0;
}

}  // namespace

BocfelHandle* bocfel_create(const char* story_path) {
    if (story_path == nullptr || story_path[0] == '\0') {
        return nullptr;
    }

    BocfelHandle* handle = new (std::nothrow) BocfelHandle();
    if (handle == nullptr) {
        return nullptr;
    }

    handle->story_path = story_path;

    if (!file_exists(story_path)) {
        handle->last_error = "story file does not exist: " + handle->story_path;
        delete handle;
        return nullptr;
    }

    return handle;
}

void bocfel_destroy(BocfelHandle* handle) {
    delete handle;
}

int bocfel_send_command(
    BocfelHandle* handle,
    const char* command,
    char* output_buffer,
    unsigned int output_buffer_len
) {
    if (handle == nullptr) {
        return -1;
    }

    if (command == nullptr) {
        set_error(handle, "command is null");
        return -1;
    }

    if (output_buffer == nullptr || output_buffer_len == 0) {
        set_error(handle, "output buffer is empty");
        return -1;
    }

    std::string script = command;
    script.push_back('\n');

    return bocfel_run_script(handle, script.c_str(), output_buffer, output_buffer_len);
}

int bocfel_run_script(
    BocfelHandle* handle,
    const char* commands,
    char* output_buffer,
    unsigned int output_buffer_len
) {
    if (handle == nullptr) {
        return -1;
    }

    if (commands == nullptr) {
        set_error(handle, "commands are null");
        return -1;
    }

    if (output_buffer == nullptr || output_buffer_len == 0) {
        set_error(handle, "output buffer is empty");
        return -1;
    }

    FILE* input = std::tmpfile();
    FILE* output = std::tmpfile();
    if (input == nullptr || output == nullptr) {
        if (input != nullptr) {
            std::fclose(input);
        }
        if (output != nullptr) {
            std::fclose(output);
        }
        set_error(handle, "failed to create temporary files for Bocfel IO");
        return -1;
    }

    if (!write_all(input, commands)) {
        std::fclose(input);
        std::fclose(output);
        set_error(handle, "failed to write commands to Bocfel input");
        return -1;
    }
    std::rewind(input);

    std::fflush(stdin);
    std::fflush(stdout);
    std::fflush(stderr);

    const int saved_stdin = duplicate_fd(file_no(stdin));
    const int saved_stdout = duplicate_fd(file_no(stdout));
    const int saved_stderr = duplicate_fd(file_no(stderr));
    if (saved_stdin == -1 || saved_stdout == -1 || saved_stderr == -1) {
        if (saved_stdin != -1) {
            close_fd(saved_stdin);
        }
        if (saved_stdout != -1) {
            close_fd(saved_stdout);
        }
        if (saved_stderr != -1) {
            close_fd(saved_stderr);
        }
        std::fclose(input);
        std::fclose(output);
        set_error(handle, "failed to save standard IO handles");
        return -1;
    }

    if (duplicate_to(file_no(input), file_no(stdin)) == -1 ||
        duplicate_to(file_no(output), file_no(stdout)) == -1 ||
        duplicate_to(file_no(output), file_no(stderr)) == -1) {
        duplicate_to(saved_stdin, file_no(stdin));
        duplicate_to(saved_stdout, file_no(stdout));
        duplicate_to(saved_stderr, file_no(stderr));
        close_fd(saved_stdin);
        close_fd(saved_stdout);
        close_fd(saved_stderr);
        std::fclose(input);
        std::fclose(output);
        set_error(handle, "failed to redirect standard IO for Bocfel");
        return -1;
    }

    std::string program = "bocfel";
    std::string story_path = handle->story_path;
    char* argv[] = {
        program.data(),
        story_path.data(),
        nullptr,
    };

    const int status = bocfel_cli_main(2, argv);

    std::fflush(stdout);
    std::fflush(stderr);
    duplicate_to(saved_stdin, file_no(stdin));
    duplicate_to(saved_stdout, file_no(stdout));
    duplicate_to(saved_stderr, file_no(stderr));
    close_fd(saved_stdin);
    close_fd(saved_stdout);
    close_fd(saved_stderr);

    const std::string output_text = read_all(output);
    std::fclose(input);
    std::fclose(output);

    if (status != 0) {
        set_error(handle, output_text.empty() ? "Bocfel exited with an error" : output_text);
        return -1;
    }

    return copy_output(handle, output_text, output_buffer, output_buffer_len);
}

const char* bocfel_last_error(BocfelHandle* handle) {
    if (handle == nullptr) {
        return "Bocfel handle is null";
    }

    return handle->last_error.c_str();
}
