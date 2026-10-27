cmake_minimum_required(VERSION 3.22)

# The build sets PEQUOD_VERSION (1.0.<build number>); PEQUOD compares it with the latest release to offer updates.
set(PEQUOD_VERSION "1.0.0" CACHE STRING "Version of this build")
project(PEQUOD_HOST VERSION ${PEQUOD_VERSION} LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Static MSVC runtime: the .exe runs without installing the VC++ redistributable.
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")

# ---------------------------------------------------------------------------
# JUCE: use a local checkout if JUCE_DIR is given, otherwise fetch it.
# ---------------------------------------------------------------------------
set(JUCE_DIR "" CACHE PATH "Path to a local JUCE checkout (optional)")

if(JUCE_DIR)
    add_subdirectory(${JUCE_DIR} JUCE)
else()
    include(FetchContent)
    FetchContent_Declare(
        JUCE
        GIT_REPOSITORY https://github.com/juce-framework/JUCE.git
        GIT_TAG        8.0.15
        GIT_SHALLOW    TRUE)
    FetchContent_MakeAvailable(JUCE)
endif()

juce_add_gui_app(PequodHost
    PRODUCT_NAME "PEQUOD Host"
    COMPANY_NAME "PEQUOD"
    BUNDLE_ID    "com.pequod.host"
    ICON_BIG     "${CMAKE_CURRENT_SOURCE_DIR}/Resources/Icon/PEQUOD-Host.png"
    ICON_SMALL   "${CMAKE_CURRENT_SOURCE_DIR}/Resources/Icon/PEQUOD-Host-small.png"
    VERSION      ${PROJECT_VERSION})

juce_generate_juce_header(PequodHost)

juce_add_binary_data(PequodFonts SOURCES
    Resources/Fonts/CourierPrime-Regular.ttf
    Resources/Fonts/CourierPrime-Bold.ttf
    Resources/Fonts/Roboto-Regular.ttf
    Resources/Fonts/Roboto-Bold.ttf
    Resources/Fonts/Ubuntu-Light.ttf
    Resources/Fonts/Hack-Regular.ttf)

target_sources(PequodHost PRIVATE
    Source/Main.cpp
    Source/HostEngine.cpp
    Source/ControlSurface.cpp
    Source/MainComponent.cpp
    Source/RackView.cpp
    Source/BrowserPanel.cpp
    Source/HeaderBar.cpp
    Source/ControlPanel.cpp
    Source/ChainMap.cpp
    Source/NativeClip.cpp
    Source/WorkerPool.cpp
    Source/RackProcessor.cpp
    Source/PresetLibrary.cpp
    Source/SettingsPanel.cpp
    Source/PluginManagerPanel.cpp
    Source/ControllerLink.cpp
    Source/ControllerManager.cpp
    Source/ControllerPanel.cpp
    Source/Updater.cpp
    Source/CrashGuard.cpp
    Source/ScreenLayout.cpp
    Source/ScreenEditor.cpp
    Source/HawthorneLink/NetSender.cpp
    Source/PequodNative.cpp
    Source/PequodPanel.cpp)

target_compile_definitions(PequodHost PRIVATE
    JUCE_PLUGINHOST_VST3=1
    JUCE_PLUGINHOST_VST=0
    JUCE_PLUGINHOST_AU=0
    JUCE_PLUGINHOST_LADSPA=0
    JUCE_PLUGINHOST_LV2=0
    JUCE_ASIO=1
    JUCE_WASAPI=1
    JUCE_DIRECTSOUND=1
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0
    JUCE_APPLICATION_NAME_STRING="$<TARGET_PROPERTY:PequodHost,JUCE_PRODUCT_NAME>"
    JUCE_APPLICATION_VERSION_STRING="$<TARGET_PROPERTY:PequodHost,JUCE_VERSION>"
    JUCE_DISPLAY_SPLASH_SCREEN=0)

# The PEQUOD engine: every PEQUOD plugin, built from the Rust source in engine/
# (cargo build --release -p pequod_engine --target x86_64-pc-windows-msvc, with RUSTFLAGS=-C target-feature=+crt-static
# to match the static runtime above). The workflow builds it before CMake runs.
set(PEQUOD_ENGINE_LIB "${CMAKE_CURRENT_SOURCE_DIR}/engine/target/x86_64-pc-windows-msvc/release/pequod_engine.lib" CACHE FILEPATH "The built PEQUOD engine library")
if(NOT EXISTS "${PEQUOD_ENGINE_LIB}")
    message(FATAL_ERROR "PEQUOD engine not built: run 'cargo build --release -p pequod_engine --target x86_64-pc-windows-msvc' in engine/ first (see engine/README.md)")
endif()

# The system libraries the engine needs, as rustc reports them (--print native-static-libs,
# saved next to the library by the build step); a fixed list if that file is missing.
set(PEQUOD_ENGINE_NATIVE_LIBS)
if(EXISTS "${PEQUOD_ENGINE_LIB}.libs")
    file(READ "${PEQUOD_ENGINE_LIB}.libs" _pq_libs)
    string(REGEX REPLACE "[ \t\r\n]+" ";" _pq_libs "${_pq_libs}")
    foreach(_l IN LISTS _pq_libs)
        if(_l MATCHES "^-l(.+)$")
            list(APPEND PEQUOD_ENGINE_NATIVE_LIBS "${CMAKE_MATCH_1}")
        elseif(_l MATCHES "^([^/].*)\\.lib$")
            list(APPEND PEQUOD_ENGINE_NATIVE_LIBS "${CMAKE_MATCH_1}")
        endif()
    endforeach()
    list(REMOVE_DUPLICATES PEQUOD_ENGINE_NATIVE_LIBS)
    list(REMOVE_ITEM PEQUOD_ENGINE_NATIVE_LIBS c msvcrt libcmt ucrt libucrt vcruntime libvcruntime)
elseif(WIN32)
    set(PEQUOD_ENGINE_NATIVE_LIBS kernel32 advapi32 bcrypt ntdll userenv ws2_32 ole32 oleaut32 shell32 user32 gdi32 opengl32 dwmapi uxtheme crypt32 secur32 propsys uuid)
else()
    set(PEQUOD_ENGINE_NATIVE_LIBS xcb GL X11-xcb X11 pthread dl m rt util)
endif()
message(STATUS "PEQUOD engine system libraries: ${PEQUOD_ENGINE_NATIVE_LIBS}")

target_link_libraries(PequodHost
    PRIVATE
        "${PEQUOD_ENGINE_LIB}"
        ${PEQUOD_ENGINE_NATIVE_LIBS}
        PequodFonts
        juce::juce_audio_utils
        juce::juce_audio_processors
        juce::juce_osc
    PUBLIC
        juce::juce_recommended_config_flags
        juce::juce_recommended_lto_flags
        juce::juce_recommended_warning_flags)
