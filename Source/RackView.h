#pragma once

#include "ControlSurface.h"
#include "PequodLookAndFeel.h"

class RackContent;

/** Callbacks the rack needs from the app (plugin windows, status messages). */
struct RackContext
{
    HostEngine& engine;
    ControlSurface& control;

    std::function<void (int uid)> openWindow;
    std::function<bool (int uid)> isWindowOpen;
    std::function<void (int uid)> closeWindow;
    std::function<void (const juce::String&)> status;
};

//==============================================================================
/** Centre panel: scrolling rack of plugin modules and A/B split modules. */
class RackView : public juce::Component
{
public:
    explicit RackView (RackContext);
    ~RackView() override;

    /** Synchronously deletes any embedded editor for this slot (call before its plugin dies). */
    void dropEditor (int slotUid);

    void collapseAll();
    void expandAll();

    void resized() override;
    void paint (juce::Graphics&) override;

private:
    RackContext ctx;
    juce::Viewport viewport;
    std::unique_ptr<RackContent> content;
};
