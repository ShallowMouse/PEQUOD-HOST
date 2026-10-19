#pragma once

#include "ControlSurface.h"
#include "PequodLookAndFeel.h"

//==============================================================================
/** Top strip: wordmark, preset navigation, learn, settings, master IN/OUT with meters. */
class HeaderBar : public juce::Component,
                  private juce::Timer
{
public:
    HeaderBar (HostEngine&, ControlSurface&);
    ~HeaderBar() override;

    void setPresetName (const juce::String& name, bool modified);

    std::function<void()> onPrev, onNext, onSave, onSaveAs, onInit, onAudioSettings, onPluginManager, onToggleLearn;
    std::function<void()> onPresetMenu;

    void paint (juce::Graphics&) override;
    void resized() override;

private:
    HostEngine& engine;
    ControlSurface& control;

    juce::TextButton prevButton { "<" }, nextButton { ">" }, presetButton,
                     saveButton { "SAVE" }, saveAsButton { "SAVE AS" }, initButton { "INIT" },
                     learnButton { "LEARN" }, audioButton { "AUDIO" }, pluginsButton { "PLUGINS" };

    LearnableSlider inKnob, outKnob;
    juce::Label inLabel, outLabel, cpuLabel;
    PeakMeter inMeter, outMeter;

    juce::Rectangle<int> wordmarkArea;

    void timerCallback() override;
    void setupKnob (LearnableSlider&, juce::Label&, const juce::String&, GainMeterProcessor*, Mapping::Target);
};
