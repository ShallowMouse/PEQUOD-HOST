#pragma once

#include "ControlSurface.h"
#include "PequodLookAndFeel.h"
#include "ChainMap.h"

//==============================================================================
/** Right-hand panel: OSC ports, MIDI inputs, input mode, and the MIDI/OSC mapping list. */
class ControlPanel : public juce::Component,
                     private juce::ListBoxModel,
                     private juce::ChangeListener,
                     private juce::Timer
{
public:
    ControlPanel (HostEngine&, ControlSurface&);
    ~ControlPanel() override;

    /** Called when a block in the effects map is clicked (slot or split uid). */
    std::function<void (int uid)> onShowInRack;

    void paint (juce::Graphics&) override;
    void resized() override;

private:
    HostEngine& engine;
    ControlSurface& control;

    juce::Label oscInLabel, oscOutLabel;
    juce::TextEditor oscInPort, oscOutHost, oscOutPort;
    juce::TextButton syncButton { "SEND SYNC" };

    juce::Label inputModeLabel;
    juce::ComboBox inputMode;

    juce::TextButton learnButton { "LEARN" }, hostLearnButton { "+ HOST" }, clearButton { "CLEAR" };
    juce::ListBox mappingList { "mappings", this };

    juce::Viewport chainViewport;
    ChainMap chainMap;

    juce::Rectangle<int> oscSection, inputSection, mapSection, chainSection;
    juce::Rectangle<int> oscInLed, oscOutLed, midiLed;

    int getNumRows() override;
    void paintListBoxItem (int row, juce::Graphics&, int w, int h, bool selected) override;
    void listBoxItemClicked (int row, const juce::MouseEvent&) override;
    void changeListenerCallback (juce::ChangeBroadcaster*) override;
    void timerCallback() override;

    void applyOsc();
    void styleLabel (juce::Label&, const juce::String&);
};
