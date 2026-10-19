#pragma once

#include "ControlSurface.h"
#include "PequodLookAndFeel.h"

//==============================================================================
/** The AUDIO window: device + MIDI + guitar input on one tab, OSC on another. */
class SettingsPanel : public juce::Component
{
public:
    SettingsPanel (HostEngine&, ControlSurface&);
    ~SettingsPanel() override;

    void paint (juce::Graphics&) override;
    void resized() override;

private:
    juce::TabbedComponent tabs { juce::TabbedButtonBar::TabsAtTop };
};
