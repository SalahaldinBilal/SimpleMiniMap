#include <algorithm>
#include <cstring>
#include <string_view>
#include <utility>
#include <vector>

#include <DynamicOutput/DynamicOutput.hpp>
#include <Helpers/String.hpp>
#include <Mod/CppUserModBase.hpp>
#include <Unreal/Core/Containers/Array.hpp>
#include <Unreal/CoreUObject/UObject/UnrealType.hpp>
#include <Unreal/Engine/UDataTable.hpp>
#include <Unreal/FField.hpp>
#include <Unreal/FProperty.hpp>
#include <Unreal/SoftObjectPtr.hpp>
#include <Unreal/FWeakObjectPtr.hpp>
#include <Unreal/Hooks/Hooks.hpp>
#include <Unreal/NameTypes.hpp>
#include <Unreal/UAssetRegistry.hpp>
#include <Unreal/UAssetRegistryHelpers.hpp>
#include <Unreal/UClass.hpp>
#include <Unreal/UFunction.hpp>
#include <Unreal/UObject.hpp>
#include <Unreal/UObjectGlobals.hpp>
#include <Unreal/UStruct.hpp>

#include "smm_host.h"

using namespace RC;
using namespace RC::Unreal;

namespace
{
    auto to_object(void* pointer) -> UObject*
    {
        return static_cast<UObject*>(pointer);
    }

    auto widen(const char* text, size_t len) -> StringType
    {
        return ensure_str(std::string_view{text, len});
    }

    void host_log(uint8_t level, const char* text, size_t len)
    {
        auto message = widen(text, len);
        switch (level)
        {
        case SMM_LOG_WARNING:
            Output::send<LogLevel::Warning>(STR("[SimpleMiniMap] {}\n"), message);
            break;
        case SMM_LOG_ERROR:
            Output::send<LogLevel::Error>(STR("[SimpleMiniMap] {}\n"), message);
            break;
        default:
            Output::send<LogLevel::Verbose>(STR("[SimpleMiniMap] {}\n"), message);
            break;
        }
    }

    template <typename Result, typename Work>
    auto guarded(const CharType* what, Result fallback, Work&& work) -> Result
    {
        try
        {
            return work();
        }
        catch (const std::exception& error)
        {
            Output::send<LogLevel::Error>(STR("[SimpleMiniMap] {} threw: {}\n"), what, ensure_str(std::string_view{error.what()}));
        }
        catch (...)
        {
            Output::send<LogLevel::Error>(STR("[SimpleMiniMap] {} threw\n"), what);
        }
        return fallback;
    }

    void* host_find_object(const char* path, size_t len)
    {
        return guarded(STR("find_object"), static_cast<void*>(nullptr), [&]() -> void* {
            auto wide = widen(path, len);
            return UObjectGlobals::StaticFindObject<UObject*>(nullptr, nullptr, wide);
        });
    }

    // Asset registry first: a hashed lookup, unlike StaticFindObject's walk of every object.
    void* host_load_object(const char* path, size_t len)
    {
        return guarded(STR("load_object"), static_cast<void*>(nullptr), [&]() -> void* {
            auto wide = widen(path, len);
            if (auto* registry = static_cast<UAssetRegistry*>(UAssetRegistryHelpers::GetAssetRegistry().ObjectPointer))
            {
                FAssetData data = registry->GetAssetByObjectPath(FName(wide.c_str(), FNAME_Add));
                if (data.PackageName().GetComparisonIndex().ToUnstableInt() != 0)
                {
                    return UAssetRegistryHelpers::GetAsset(data);
                }
            }
            return UObjectGlobals::StaticFindObject<UObject*>(nullptr, nullptr, wide);
        });
    }

    int32_t host_read_render_target(void* world, void* render_target, uint32_t* out_pixels, int32_t capacity)
    {
        return guarded(STR("read_render_target"), int32_t{-1}, [&]() -> int32_t {
            static auto* library = UObjectGlobals::StaticFindObject<UObject*>(nullptr, nullptr, STR("/Script/Engine.Default__KismetRenderingLibrary"));
            static auto* function = UObjectGlobals::StaticFindObject<UFunction*>(nullptr, nullptr, STR("/Script/Engine.KismetRenderingLibrary:ReadRenderTarget"));
            if (!library || !function)
            {
                return -1;
            }
            auto offset = [&](const CharType* name) -> int32 {
                auto* property = function->FindProperty(FName(name, FNAME_Find));
                return property ? property->GetOffset_Internal() : -1;
            };
            const int32 world_at = offset(STR("WorldContextObject"));
            const int32 target_at = offset(STR("TextureRenderTarget"));
            const int32 samples_at = offset(STR("OutSamples"));
            const int32 normalize_at = offset(STR("bNormalize"));
            if (world_at < 0 || target_at < 0 || samples_at < 0 || normalize_at < 0)
            {
                return -1;
            }
            std::vector<uint64_t> storage((function->GetParmsSize() + 7) / 8, 0);
            auto* params = reinterpret_cast<uint8_t*>(storage.data());
            std::memcpy(params + world_at, &world, sizeof(world));
            std::memcpy(params + target_at, &render_target, sizeof(render_target));
            params[normalize_at] = 1;
            library->ProcessEvent(function, params);

            auto* samples = reinterpret_cast<TArray<uint32>*>(params + samples_at);
            const int32 count = samples->Num();
            std::memcpy(out_pixels, samples->GetData(), static_cast<size_t>(std::min(count, capacity)) * sizeof(uint32_t));
            samples->~TArray<uint32>();
            return count;
        });
    }

    void* host_construct_object(void* uclass, void* outer)
    {
        return guarded(STR("construct_object"), static_cast<void*>(nullptr), [&]() -> void* {
            FStaticConstructObjectParameters params{static_cast<UClass*>(uclass), to_object(outer)};
            return UObjectGlobals::StaticConstructObject(params);
        });
    }

    void* host_find_function(void* object_or_struct, const char* name, size_t len)
    {
        return guarded(STR("find_function"), static_cast<void*>(nullptr), [&]() -> void* {
            auto wide = widen(name, len);
            return to_object(object_or_struct)->GetFunctionByNameInChain(FName(wide.c_str(), FNAME_Find));
        });
    }

    int32_t host_function_params_size(void* function)
    {
        return guarded(STR("function_params_size"), int32_t{-1}, [&]() -> int32_t {
            return static_cast<int32_t>(static_cast<UFunction*>(function)->GetParmsSize());
        });
    }

    int32_t host_property_offset(void* ustruct, const char* name, size_t len)
    {
        return guarded(STR("property_offset"), int32_t{-1}, [&]() -> int32_t {
            auto wide = widen(name, len);
            auto* property = static_cast<UStruct*>(ustruct)->FindProperty(FName(wide.c_str(), FNAME_Find));
            return property ? property->GetOffset_Internal() : -1;
        });
    }

    void host_process_event(void* object, void* function, void* params)
    {
        guarded(STR("process_event"), 0, [&]() -> int {
            to_object(object)->ProcessEvent(static_cast<UFunction*>(function), params);
            return 0;
        });
    }

    void host_weak_make(void* object, int32_t* out_index_and_serial)
    {
        FWeakObjectPtr weak{to_object(object)};
        out_index_and_serial[0] = weak.ObjectIndex;
        out_index_and_serial[1] = weak.ObjectSerialNumber;
    }

    void* host_weak_get(int32_t index, int32_t serial)
    {
        FWeakObjectPtr weak{};
        weak.ObjectIndex = index;
        weak.ObjectSerialNumber = serial;
        return weak.Get();
    }

    size_t host_object_name(void* object, char* buffer, size_t capacity)
    {
        return guarded(STR("object_name"), size_t{0}, [&]() -> size_t {
            auto name = to_utf8_string(to_object(object)->GetName());
            auto count = std::min(name.size(), capacity);
            std::memcpy(buffer, name.data(), count);
            return count;
        });
    }

    void* host_object_class(void* object)
    {
        return to_object(object)->GetClassPrivate();
    }

    static_assert(sizeof(FName) == sizeof(uint64_t));

    uint64_t host_make_name(const char* text, size_t len)
    {
        return guarded(STR("make_name"), uint64_t{0}, [&]() -> uint64_t {
            auto wide = widen(text, len);
            FName name(wide.c_str(), FNAME_Add);
            uint64_t packed{};
            std::memcpy(&packed, &name, sizeof(packed));
            return packed;
        });
    }

    int32_t host_map_entries(void* ustruct, const char* name, size_t len, void* container, void** out_pairs, int32_t capacity, int32_t* out_value_offset)
    {
        return guarded(STR("map_entries"), int32_t{-1}, [&]() -> int32_t {
            auto wide = widen(name, len);
            auto* map_property = CastField<FMapProperty>(static_cast<UStruct*>(ustruct)->FindProperty(FName(wide.c_str(), FNAME_Find)));
            if (!map_property)
            {
                return -1;
            }
            auto* key = map_property->GetKeyProp();
            auto* value = map_property->GetValueProp();
            auto layout = FScriptMap::GetScriptLayout(key->GetSize(), key->GetMinAlignment(), value->GetSize(), value->GetMinAlignment());
            auto* map = map_property->ContainerPtrToValuePtr<FScriptMap>(container);
            int32_t count = 0;
            for (int32 index = 0; index < map->GetMaxIndex(); ++index)
            {
                if (!map->IsValidIndex(index))
                {
                    continue;
                }
                if (count < capacity)
                {
                    out_pairs[count] = map->GetData(index, layout);
                }
                ++count;
            }
            *out_value_offset = layout.ValueOffset;
            return count;
        });
    }

    int32_t host_data_table_rows(void* table, void** out_rows, int32_t capacity)
    {
        return guarded(STR("data_table_rows"), int32_t{-1}, [&]() -> int32_t {
            int32_t count = 0;
            for (auto& pair : static_cast<UDataTable*>(table)->GetRowMap())
            {
                if (count < capacity)
                {
                    out_rows[count] = pair.Value;
                }
                ++count;
            }
            return count;
        });
    }

    auto copy_utf8(const StringType& text, char* buffer, size_t capacity) -> size_t
    {
        auto utf8 = to_utf8_string(text);
        auto count = std::min(utf8.size(), capacity);
        std::memcpy(buffer, utf8.data(), count);
        return count;
    }

    size_t host_soft_object_path(const void* soft_object_ptr, char* buffer, size_t capacity)
    {
        return guarded(STR("soft_object_path"), size_t{0}, [&]() -> size_t {
            auto path = static_cast<const FSoftObjectPtr*>(soft_object_ptr)->ToString();
            return copy_utf8(StringType{*path}, buffer, capacity);
        });
    }

    size_t host_name_string(uint64_t packed, char* buffer, size_t capacity)
    {
        return guarded(STR("name_string"), size_t{0}, [&]() -> size_t {
            FName name{};
            std::memcpy(&name, &packed, sizeof(packed));
            return copy_utf8(name.ToString(), buffer, capacity);
        });
    }

    bool host_is_a(void* object, void* uclass)
    {
        return guarded(STR("is_a"), false, [&]() -> bool {
            return to_object(object)->IsA(static_cast<UClass*>(uclass));
        });
    }

    // Through the property, so bitfield flags such as bShowMouseCursor get the right bit.
    bool host_set_bool_property(void* object, const char* name, size_t len, bool value)
    {
        return guarded(STR("set_bool_property"), false, [&]() -> bool {
            auto wide = widen(name, len);
            auto* property = CastField<FBoolProperty>(to_object(object)->GetClassPrivate()->FindProperty(FName(wide.c_str(), FNAME_Find)));
            if (!property)
            {
                return false;
            }
            property->SetPropertyValueInContainer(object, value);
            return true;
        });
    }

    // The same for a bool inside a struct value, such as the bOverride_ flags of FPostProcessSettings.
    bool host_set_struct_bool(void* ustruct, void* container, const char* name, size_t len, bool value)
    {
        return guarded(STR("set_struct_bool"), false, [&]() -> bool {
            auto wide = widen(name, len);
            auto* property = CastField<FBoolProperty>(static_cast<UStruct*>(ustruct)->FindProperty(FName(wide.c_str(), FNAME_Find)));
            if (!property)
            {
                return false;
            }
            property->SetPropertyValueInContainer(container, value);
            return true;
        });
    }

    void host_bind_key(uint8_t key, uint8_t modifiers, uint32_t action);

    void* host_find_instance(const char* class_name, size_t len)
    {
        return guarded(STR("find_instance"), static_cast<void*>(nullptr), [&]() -> void* {
            std::vector<UObject*> found{};
            UObjectGlobals::FindAllOf(std::string_view{class_name, len}, found);
            for (auto* object : found)
            {
                if (object && !object->GetName().starts_with(STR("Default__")))
                {
                    return object;
                }
            }
            return nullptr;
        });
    }

    const SmmHostApi host_api{
        .version = SMM_HOST_API_VERSION,
        .log = &host_log,
        .find_object = &host_find_object,
        .load_object = &host_load_object,
        .construct_object = &host_construct_object,
        .find_function = &host_find_function,
        .function_params_size = &host_function_params_size,
        .property_offset = &host_property_offset,
        .process_event = &host_process_event,
        .weak_make = &host_weak_make,
        .weak_get = &host_weak_get,
        .object_name = &host_object_name,
        .object_class = &host_object_class,
        .make_name = &host_make_name,
        .map_entries = &host_map_entries,
        .data_table_rows = &host_data_table_rows,
        .soft_object_path = &host_soft_object_path,
        .name_string = &host_name_string,
        .is_a = &host_is_a,
        .bind_key = &host_bind_key,
        .find_instance = &host_find_instance,
        .read_render_target = &host_read_render_target,
        .set_bool_property = &host_set_bool_property,
        .set_struct_bool = &host_set_struct_bool,
    };
} // namespace

class SimpleMiniMap final : public CppUserModBase
{
  public:
    SimpleMiniMap()
    {
        ModName = STR("SimpleMiniMap");
        ModVersion = STR("0.1.1");
        ModDescription = STR("A minimap for Palworld");
        ModAuthors = STR("SalahaldinBilal");
        s_instance = this;
    }

    auto bind(uint8_t key, uint8_t modifiers, uint32_t action) -> void
    {
        auto callback = [action]() {
            smm_input(action);
        };
        if (modifiers == 0)
        {
            register_keydown_event(static_cast<Input::Key>(key), callback);
            return;
        }
        Input::Handler::ModifierKeyArray held{};
        size_t count = 0;
        const std::pair<uint8_t, Input::ModifierKey> known[] = {
                {SMM_MODIFIER_SHIFT, Input::ModifierKey::SHIFT},
                {SMM_MODIFIER_CONTROL, Input::ModifierKey::CONTROL},
                {SMM_MODIFIER_ALT, Input::ModifierKey::ALT},
        };
        for (const auto& [bit, modifier] : known)
        {
            if ((modifiers & bit) != 0 && count < held.size())
            {
                held[count++] = modifier;
            }
        }
        register_keydown_event(static_cast<Input::Key>(key), held, callback);
    }

    static inline SimpleMiniMap* s_instance{nullptr};

    ~SimpleMiniMap() override
    {
        s_instance = nullptr;
        for (auto id : m_callbacks)
        {
            Hook::UnregisterCallback(id);
        }
        if (m_started)
        {
            smm_shutdown();
        }
    }

    auto on_unreal_init() -> void override
    {
        if (!smm_init(&host_api))
        {
            Output::send<LogLevel::Error>(STR("[SimpleMiniMap] core failed to start, mod disabled\n"));
            return;
        }
        m_started = true;

        m_callbacks.push_back(Hook::RegisterEngineTickPostCallback(
                [](auto&, UEngine* engine, float delta_seconds, bool) {
                    smm_tick(engine, delta_seconds);
                },
                {false, false, STR("SimpleMiniMap"), STR("Tick")}));

        m_callbacks.push_back(Hook::RegisterLoadMapPreCallback(
                [](auto&&...) {
                    smm_world_unload();
                },
                {false, false, STR("SimpleMiniMap"), STR("WorldUnload")}));

        m_callbacks.push_back(Hook::RegisterBeginPlayPostCallback(
                [](auto&, AActor* actor) {
                    smm_actor_begin(actor);
                },
                {false, true, STR("SimpleMiniMap"), STR("ActorBegin")}));

        m_callbacks.push_back(Hook::RegisterEndPlayPreCallback(
                [](auto&, AActor* actor, auto) {
                    smm_actor_end(actor);
                },
                {false, true, STR("SimpleMiniMap"), STR("ActorEnd")}));
    }

  private:
    std::vector<Hook::GlobalCallbackId> m_callbacks{};
    bool m_started{false};
};

namespace
{
    void host_bind_key(uint8_t key, uint8_t modifiers, uint32_t action)
    {
        if (SimpleMiniMap::s_instance)
        {
            SimpleMiniMap::s_instance->bind(key, modifiers, action);
        }
    }
} // namespace

#define SMM_API __declspec(dllexport)
extern "C"
{
    SMM_API CppUserModBase* start_mod()
    {
        return new SimpleMiniMap();
    }

    SMM_API void uninstall_mod(CppUserModBase* mod)
    {
        delete mod;
    }
}
