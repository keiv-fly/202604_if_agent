#include "bocfel_embed.h"

#include <cstring>
#include <fstream>
#include <new>
#include <string>

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

    const std::string message =
        "embedded Bocfel source is not vendored yet; cannot execute command: " +
        std::string(command);

    if (message.size() + 1 > output_buffer_len) {
        set_error(handle, "output buffer is too small");
        return 1;
    }

    std::memcpy(output_buffer, message.c_str(), message.size() + 1);
    set_error(handle, message);
    return -1;
}

const char* bocfel_last_error(BocfelHandle* handle) {
    if (handle == nullptr) {
        return "Bocfel handle is null";
    }

    return handle->last_error.c_str();
}
