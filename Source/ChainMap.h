#pragma once

#include "HostEngine.h"
#include "PequodLookAndFeel.h"

//==============================================================================
/** Overview of the signal chain: IN -> modules (splits as two columns) -> OUT.
    Click a box to jump to it in the rack; click its LED to switch it on/off. */
class ChainMap : public juce::Component,
                 public juce::SettableTooltipClient,
                 private juce::ChangeListener,
                 private juce::Timer
{
public:
    explicit ChainMap (HostEngine&);
    ~ChainMap() override;

    std::function<void (int slotOrItemUid)> onSelect;
    std::function<bool()> isMuted;

    /** Height needed for the current rack at the given width. */
    int getDesiredHeight() const { return desiredHeight; }

    void paint (juce::Graphics&) override;
    void resized() override { rebuild(); }
    void mouseMove (const juce::MouseEvent&) override;
    void mouseExit (const juce::MouseEvent&) override;
    void mouseUp (const juce::MouseEvent&) override;

private:
    struct Block
    {
        enum class Kind { input, output, plugin, splitHeader } kind;
        juce::Rectangle<float> bounds;
        int uid = 0;            // slot uid (plugin) or item uid (split)
        int number = 0;
        juce::String name;
        bool bypassed = false;
    };

    HostEngine& engine;
    std::vector<Block> blocks;
    std::vector<juce::Line<float>> wires;
    int desiredHeight = 0;
    int hover = -1;
    bool lastMuted = false;

    void rebuild();
    int blockAt (juce::Point<float>) const;
    static juce::Rectangle<float> ledArea (const Block& b) { return b.bounds.withWidth (20.0f); }

    void changeListenerCallback (juce::ChangeBroadcaster*) override { rebuild(); }
    void timerCallback() override;
};
