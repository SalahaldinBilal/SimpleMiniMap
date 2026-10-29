set(TARGET SimpleMiniMap)

set(SMM_CORE_DIR "${CMAKE_SOURCE_DIR}/core")
set(SMM_CORE_LIB "${SMM_CORE_DIR}/target/release/simpleminimap_core.lib")
set(SMM_GAME_MODS_DIR "C:/Program Files (x86)/Steam/steamapps/common/Palworld/Mods/NativeMods/UE4SS/Mods"
    CACHE PATH "UE4SS Mods folder the built mod is copied into")
option(SMM_DEPLOY "Copy the built mod into SMM_GAME_MODS_DIR" ON)

add_custom_target(SimpleMiniMapCore
    COMMAND cargo build --release --manifest-path "${SMM_CORE_DIR}/Cargo.toml"
    WORKING_DIRECTORY "${SMM_CORE_DIR}"
    BYPRODUCTS "${SMM_CORE_LIB}"
    USES_TERMINAL
)

add_library(${TARGET} SHARED src/dllmain.cpp)
add_dependencies(${TARGET} SimpleMiniMapCore)
target_include_directories(${TARGET} PRIVATE include)
target_link_libraries(${TARGET} PRIVATE
    "${SMM_CORE_LIB}"
    kernel32 advapi32 ntdll userenv ws2_32 bcrypt dbghelp
)
target_link_libraries(${TARGET} PUBLIC UE4SS)
set_target_properties(${TARGET} PROPERTIES OUTPUT_NAME main)

if(SMM_DEPLOY)
    add_custom_command(TARGET ${TARGET} POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E make_directory "${SMM_GAME_MODS_DIR}/${TARGET}/dlls"
        COMMAND ${CMAKE_COMMAND} -E copy_if_different $<TARGET_FILE:${TARGET}> "${SMM_GAME_MODS_DIR}/${TARGET}/dlls/main.dll"
    )
endif()
