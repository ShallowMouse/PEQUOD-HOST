#pragma once

#include <JuceHeader.h>

//==============================================================================
/** Notices when PEQUOD didn't close properly last time, and which plugin it was in when it crashed.

    - A "running" marker file exists while PEQUOD is open; finding it at start means the last run crashed.
    - Code that calls into a plugin wraps the call in a Scope. If the process crashes inside it (Windows),
      the plugin's name is written to a report file, so the next start can offer to leave that plugin out.
    - Crashes while a plugin processes audio are caught where they happen (RackProcessor) and don't reach here. */
namespace CrashGuard
{
    struct LastRun
    {
        bool crashed = false;
        juce::String plugin;   // empty when it isn't known
        juce::String doing;    // "loading", "opening its window"...
    };

    /** Reads what happened last time, then marks this run as started and installs the crash handler. */
    LastRun start (const juce::File& folder);

    /** Call when PEQUOD closes normally. */
    void cleanExit();

    /** Marks the current thread as being inside a plugin. Names must stay valid while the scope lives. */
    struct Scope
    {
        Scope (const char* plugin, const char* doing) noexcept;
        ~Scope() noexcept;
        const char* prevPlugin;
        const char* prevDoing;
    };
}
