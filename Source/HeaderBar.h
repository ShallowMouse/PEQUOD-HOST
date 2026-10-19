#pragma once

#include "ControlSurface.h"
#include "PequodLookAndFeel.h"
#include "Tuner.h"

//==============================================================================
/** Compact note + cents display fed by the Tuner. */
class TunerDisplay : public juce::Component,
                     public juce::SettableTooltipClient,
                     private juce::Timer
{
public:
    explicit TunerDisplay (Tuner& t) : tuner (t)
    {
        setTooltip ("Tuner - follows the guitar input. Right-click to change the reference pitch.");
        startTimerHz (30);
    }

    void paint (juce::Graphics&) override;
    void mouseDown (const juce::MouseEvent&) override;

private:
    Tuner& tuner;
    bool lastValid = false;
    float lastCents = 0.0f;
    int lastNote = -1;

    void timerCallback() override;
};

//==============================================================================
/** Top strip: wordmark, preset navigation, learn, tuner + mute, settings, master IN/OUT with meters. */
class HeaderBar : public juce::Component,
                  private juce::Timer
{
public:
    HeaderBar (HostEngine&, ControlSurface&, Tuner&);
    ~HeaderBar() override;

    void setPresetName (const juce::String& name, bool modified);

    std::function<void()> onPrev, onNext, onSave, onSaveAs, onInit, onAudioSettings, onPluginManager, onToggleLearn;
    std::function<void()> onPresetMenu;
    std::function<void (int stages)> onPipelineStagesChanged;

    void paint (juce::Graphics&) override;
    void resized() override;

private:
    HostEngine& engine;
    ControlSurface& control;

    juce::TextButton prevButton { "<" }, nextButton { ">" }, presetButton,
                     saveButton { "SAVE" }, saveAsButton { "SAVE AS" }, initButton { "INIT" },
                     learnButton { "LEARN" }, audioButton { "AUDIO" }, pluginsButton { "PLUGINS" };

    TunerDisplay tunerDisplay;
    LearnableButton muteButton { "MUTE" };

    LearnableSlider inKnob, outKnob;
    juce::Label inLabel, outLabel;

    /** CPU meter that doubles as the multi-core setting. */
    struct CpuButton : public juce::Button
    {
        CpuButton() : Button ("cpu") {}
        juce::String top, bottom;
        bool hot = false;
        void paintButton (juce::Graphics&, bool over, bool down) override;
    };
    CpuButton cpuButton;
    void showCoresMenu();
    PeakMeter inMeter, outMeter;

    juce::Rectangle<int> wordmarkArea;

    void timerCallback() override;
    void setupKnob (LearnableSlider&, juce::Label&, const juce::String&, GainMeterProcessor*, Mapping::Target);
};
