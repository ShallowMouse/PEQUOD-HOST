#include "NativeClip.h"
#include <map>

#if JUCE_WINDOWS
 #ifndef NOMINMAX
  #define NOMINMAX
 #endif
 #include <windows.h>
#endif

namespace NativeClip
{
   #if JUCE_WINDOWS
    static void collect (juce::Component& c, juce::Array<juce::HWNDComponent*>& out)
    {
        if (auto* h = dynamic_cast<juce::HWNDComponent*> (&c))
            out.add (h);
        for (auto* child : c.getChildren())
            collect (*child, out);
    }
   #endif

    void clipToArea (juce::Component& root, juce::Rectangle<int> visibleScreenArea)
    {
       #if JUCE_WINDOWS
        juce::Array<juce::HWNDComponent*> natives;
        collect (root, natives);

        const auto visible = juce::Desktop::getInstance().getDisplays().logicalToPhysical (visibleScreenArea);

        for (auto* nc : natives)
        {
            auto hwnd = static_cast<HWND> (nc->getHWND());
            if (hwnd == nullptr) continue;

            RECT wr;
            if (! GetWindowRect (hwnd, &wr)) continue;

            const juce::Rectangle<int> window (wr.left, wr.top, wr.right - wr.left, wr.bottom - wr.top);
            const auto shown = window.getIntersection (visible);

            // Only touch the window when its visible part actually changed (avoids redraw flicker).
            static std::map<HWND, juce::Rectangle<int>> lastApplied;
            const auto key = shown.isEmpty() ? juce::Rectangle<int>() : shown.translated (-window.getX(), -window.getY());
            if (auto found = lastApplied.find (hwnd); found != lastApplied.end() && found->second == key)
                continue;
            lastApplied[hwnd] = key;

            if (shown == window)
            {
                SetWindowRgn (hwnd, nullptr, TRUE);           // fully visible: no clipping
            }
            else
            {
                const auto local = shown.translated (-window.getX(), -window.getY());
                auto rgn = shown.isEmpty() ? CreateRectRgn (0, 0, 0, 0)
                                           : CreateRectRgn (local.getX(), local.getY(), local.getRight(), local.getBottom());
                SetWindowRgn (hwnd, rgn, TRUE);              // the window takes ownership of rgn
            }
        }
       #elif JUCE_LINUX
        // XEmbed windows can't be partially clipped cheaply: hide any that aren't fully in view.
        std::function<void (juce::Component&)> visit = [&] (juce::Component& c)
        {
            if (auto* x = dynamic_cast<juce::XEmbedComponent*> (&c))
            {
                const bool inView = visibleScreenArea.contains (x->getScreenBounds());
                if (x->isVisible() != inView)
                    x->setVisible (inView);
                return;
            }
            for (auto* child : c.getChildren())
                visit (*child);
        };
        visit (root);
       #else
        juce::ignoreUnused (root, visibleScreenArea);
       #endif
    }
}
