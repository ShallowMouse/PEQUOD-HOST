#pragma once

// MELVILLE: when this program is built into MELVILLE (MELVILLE_HOSTED), it opens MELVILLE's
// device for its part instead of picking an interface, leaves the device settings it saves
// alone, and has no audio settings of its own (MELVILLE's INTERFACE does that). Built on its own (the normal build), all of this does nothing.

#include <juce_audio_devices/juce_audio_devices.h>
#include <juce_gui_basics/juce_gui_basics.h>

#if defined (MELVILLE_HOSTED) && MELVILLE_HOSTED
 #include <MelvilleHosted.h>
#else
namespace melville
{
    inline bool isHosted() noexcept { return false; }
    inline bool attach (juce::AudioDeviceManager&, const char*) { return false; }
    inline std::unique_ptr<juce::Component> midiInputsPanel (juce::AudioDeviceManager&) { return {}; }
}
#endif
