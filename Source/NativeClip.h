#pragma once

#include <JuceHeader.h>

/** Plugin panels are native child windows on Windows, which ignore JUCE's own clipping and would
    draw on top of the header when the rack is scrolled. This clips each native window inside
    `root` to the visible part of the rack. No-op on other platforms. */
namespace NativeClip
{
    void clipToArea (juce::Component& root, juce::Rectangle<int> visibleScreenArea);
}
