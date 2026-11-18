#pragma once

#include <juce_gui_basics/juce_gui_basics.h>

/** A tooltip window that speaks only for the components it's nearest to. JUCE's own shows a tip
    for anything in its window, so two in one window (a part inside a host, a panel inside a part)
    showed every tip twice: this one stays quiet unless it's the first tooltip window found going
    up from the component under the mouse. (The same file in every part of the studio.) */
class NearestTooltips : public juce::TooltipWindow
{
public:
    NearestTooltips (juce::Component* parent, int delayMs) : juce::TooltipWindow (parent, delayMs) {}

    juce::String getTipFor (juce::Component& c) override
    {
        return nearestFor (c) == this ? juce::TooltipWindow::getTipFor (c) : juce::String();
    }

    /** The tooltip window nearest above `c` (nullptr: none - the studio's own then speaks). */
    static juce::TooltipWindow* nearestFor (juce::Component& c)
    {
        for (auto* p = &c; p != nullptr; p = p->getParentComponent())
            for (auto* child : p->getChildren())
                if (auto* t = dynamic_cast<juce::TooltipWindow*> (child))
                    return t;
        return nullptr;
    }
};
