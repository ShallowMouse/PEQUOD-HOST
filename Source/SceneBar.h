#pragma once

#include "HostEngine.h"
#include "ControlSurface.h"
#include "PequodLookAndFeel.h"

//==============================================================================
/** The bar above the modifiers: this preset's SCENES (versions of the preset switched instantly)
    and its own PRESET IN / PRESET OUT levels. The header knobs are the master levels. */
class SceneBar : public juce::Component,
                 private juce::Timer
{
public:
    SceneBar (HostEngine&, ControlSurface&);
    ~SceneBar() override;

    static constexpr int height = 46;

    /** Right-click on PRESET OUT adds these (level matching lives in the header). */
    std::function<void (juce::PopupMenu&)> addOutMenuItems;
    std::function<void (const juce::String&)> onStatus;
    void showMeasuring (int secondsLeft);   // -1 = not measuring

    void rebuild();
    void mouseDown (const juce::MouseEvent&) override;
    void paint (juce::Graphics&) override;
    void resized() override;

private:
    HostEngine& engine;
    ControlSurface& control;
    juce::OwnedArray<juce::TextButton> sceneButtons;
    juce::TextButton addButton { "+ SCENE" };
    LearnableSlider presetIn, presetOut;
    juce::Label inLabel, outLabel;
    int builtCount = -1, builtCurrent = -2;
    bool suppressClick = false;
    juce::StringArray builtNames;

    void sceneMenu (int index);
    void timerCallback() override;
};
