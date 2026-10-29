#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define SMM_HOST_API_VERSION 9

enum SmmModifier
{
    SMM_MODIFIER_SHIFT = 1,
    SMM_MODIFIER_CONTROL = 2,
    SMM_MODIFIER_ALT = 4,
};

enum SmmLogLevel
{
    SMM_LOG_INFO = 0,
    SMM_LOG_WARNING = 1,
    SMM_LOG_ERROR = 2,
};

// Field order must match HostApi in core/src/host.rs.
typedef struct SmmHostApi
{
    uint32_t version;
    void (*log)(uint8_t level, const char* text, size_t len);
    void* (*find_object)(const char* path, size_t len);
    void* (*load_object)(const char* path, size_t len);
    void* (*construct_object)(void* uclass, void* outer);
    void* (*find_function)(void* object_or_struct, const char* name, size_t len);
    int32_t (*function_params_size)(void* function);
    int32_t (*property_offset)(void* ustruct, const char* name, size_t len);
    void (*process_event)(void* object, void* function, void* params);
    void (*weak_make)(void* object, int32_t* out_index_and_serial);
    void* (*weak_get)(int32_t index, int32_t serial);
    size_t (*object_name)(void* object, char* buffer, size_t capacity);
    void* (*object_class)(void* object);
    uint64_t (*make_name)(const char* text, size_t len);
    int32_t (*map_entries)(void* ustruct, const char* property, size_t len, void* container, void** out_pairs, int32_t capacity, int32_t* out_value_offset);
    int32_t (*data_table_rows)(void* table, void** out_rows, int32_t capacity);
    size_t (*soft_object_path)(const void* soft_object_ptr, char* buffer, size_t capacity);
    size_t (*name_string)(uint64_t name, char* buffer, size_t capacity);
    bool (*is_a)(void* object, void* uclass);
    void (*bind_key)(uint8_t key, uint8_t modifiers, uint32_t action);
    void* (*find_instance)(const char* class_name, size_t len);
    int32_t (*read_render_target)(void* world, void* render_target, uint32_t* out_pixels, int32_t capacity);
    bool (*set_bool_property)(void* object, const char* name, size_t len, bool value);
    bool (*set_struct_bool)(void* ustruct, void* container, const char* name, size_t len, bool value);
} SmmHostApi;

#ifdef __cplusplus
extern "C"
{
#endif

    bool smm_init(const SmmHostApi* api);
    void smm_tick(void* engine, float delta_seconds);
    void smm_world_unload(void);
    void smm_shutdown(void);
    void smm_input(uint32_t action);
    void smm_actor_begin(void* actor);
    void smm_actor_end(void* actor);

#ifdef __cplusplus
}
#endif
