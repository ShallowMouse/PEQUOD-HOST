#pragma once

#include "HostEngine.h"
#include "PequodLookAndFeel.h"

//==============================================================================
/** The MODIFIERS strip above the rack: LFOs, envelope followers, triggered envelopes and step
    sequencers, each a small module with its controls, a live scope, LEARN, and the knobs it moves. */
class ModifierRow : public juce::Component,
                    private juce::Timer
{
public:
    explicit ModifierRow (HostEngine&);
    ~ModifierRow() override;

    /** Height wanted: just the title bar when hidden. */
    int getIdealHeight() const { return expanded ? barHeight + moduleHeight + 12 : barHeight; }
    bool isExpanded() const { return expanded; }
    void setExpanded (bool);
    std::function<void()> onHeightChanged;

    /** The engine's modifiers changed (preset loaded, added, removed...). */
    void rebuild();

    void paint (juce::Graphics&) override;
    void resized() override;

    static constexpr int barHeight = 30, moduleHeight = 236;
    static juce::Colour colourFor (int type);

    class Module;

private:
    HostEngine& engine;
    bool expanded = true;

    juce::TextButton showButton { "HIDE" };
    juce::TextButton addLfo { "+ LFO" }, addFollower { "+ FOLLOWER" }, addEnvelope { "+ ENVELOPE" }, addSteps { "+ STEPS" };
    juce::Viewport viewport;
    juce::Component strip;
    juce::OwnedArray<Module> modules;
    int builtCount = -1;

    void add (Modifiers::Type);
    void timerCallback() override;
};
