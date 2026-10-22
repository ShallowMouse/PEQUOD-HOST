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
    Resources/Fonts/Roboto-Bold.ttf)

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
    Source/CrashGuard.cpp)

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

target_link_libraries(PequodHost
    PRIVATE
        PequodFonts
        juce::juce_audio_utils
        juce::juce_audio_processors
        juce::juce_osc
    PUBLIC
        juce::juce_recommended_config_flags
        juce::juce_recommended_lto_flags
        juce::juce_recommended_warning_flags)
