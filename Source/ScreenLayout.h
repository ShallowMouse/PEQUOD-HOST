#pragma once

#include <JuceHeader.h>

//==============================================================================
/** A layout for the controller's 128 x 64 screen, made in PEQUOD's screen editor.
    Text can hold {tokens} that PEQUOD fills in (preset name, BPM...). The controller
    only draws the finished items it's sent (firmware 2.2+), so the editor's preview is exact. */
namespace ScreenLayout
{
    constexpr int width = 128, height = 64;

    struct Element
    {
        enum class Kind { text, box, frame };
        Kind kind = Kind::text;
        juce::String text;        // text: may contain {tokens}
        int x = 0, y = 0;         // text: x is the left edge, centre or right edge (align)
        int w = 128, h = 1;       // box / frame size
        int size = 1;             // text: 1-4 (each size is 6 x 8 pixels per letter)
        int align = 1;            // 0 left, 1 centre, 2 right
        bool inverted = false;    // text: black on a white box
        bool fit = true;          // text: shrink (then cut) to fit the screen

        juce::var toVar() const;
        static Element fromVar (const juce::var&);
    };

    struct Layout
    {
        std::vector<Element> elements;
        bool messageBar = true;   // the white message bar when a switch is pressed (else only {msg} shows it)

        juce::var toVar() const;
        static Layout fromVar (const juce::var&);
        static Layout standard();  // looks like the built-in screen
    };

    /** A finished item, as sent to the controller and drawn by it. */
    struct Item
    {
        char kind = 't';          // 't' text, 'b' box, 'r' frame
        int x = 0, y = 0, w = 0, h = 0, size = 1, align = 0;
        bool inverted = false;
        juce::String text;
        int element = -1;         // which layout element it came from
        juce::Rectangle<int> bounds() const;     // where it lands on the screen
    };

    /** Fills in the tokens and fits the text. lookup gets the token's name and number ("fx", 2). */
    std::vector<Item> resolve (const Layout&, const std::function<juce::String (const juce::String& name, int number)>& lookup);
    juce::var itemsToVar (const std::vector<Item>&);

    /** Draws the items exactly as the controller does (same font, same pixels). */
    juce::Image render (const std::vector<Item>&);

    struct Token { const char* text; const char* description; };
    const std::vector<Token>& tokens();
}
