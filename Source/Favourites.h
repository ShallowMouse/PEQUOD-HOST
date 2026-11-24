#pragma once

#include <JuceHeader.h>

//==============================================================================
/** A browser's favourites: a set of plugin keys (PluginTags::duplicateKey), saved by the app that
    owns the browser (IRVING and ALCOTT each keep their own), and the star drawn beside each row. */
struct Favourites
{
    juce::StringArray keys;

    bool contains (const juce::String& k) const { return keys.contains (k); }
    void toggle (const juce::String& k)
    {
        if (keys.contains (k)) keys.removeString (k);
        else keys.addIfNotAlreadyThere (k);
    }
    juce::String toString() const               { return keys.joinIntoString ("\n"); }
    void fromString (const juce::String& s)     { keys = juce::StringArray::fromLines (s); keys.removeEmptyStrings(); }

    /** The width a row gives its star, on the right. */
    static constexpr int starWidth = 26;

    /** A five-pointed star, filled when it's a favourite, an outline when not (brighter on hover). */
    static void drawStar (juce::Graphics& g, juce::Rectangle<float> area, bool on, juce::Colour colour, bool hover = false)
    {
        juce::Path p;
        const float r = juce::jmin (area.getWidth(), area.getHeight()) * 0.36f;
        p.addStar (area.getCentre(), 5, r * 0.45f, r, 0.0f);
        if (on)
        {
            g.setColour (colour);
            g.fillPath (p);
        }
        else
        {
            g.setColour (colour.withAlpha (hover ? 0.8f : 0.35f));
            g.strokePath (p, juce::PathStrokeType (1.2f));
        }
    }
};
