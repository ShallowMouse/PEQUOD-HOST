#pragma once

// MELVILLE: when this program is built into MELVILLE (MELVILLE_HOSTED), it opens MELVILLE's
// device for its part instead of picking an interface, and leaves the device settings it
// saves alone. Built on its own (the normal build), all of this does nothing.

#include <juce_audio_devices/juce_audio_devices.h>

#if defined (MELVILLE_HOSTED) && MELVILLE_HOSTED
 #include <MelvilleHosted.h>
#else
namespace melville
{
    inline bool isHosted() noexcept { return false; }
    inline bool attach (juce::AudioDeviceManager&, const char*) { return false; }
}
#endif
