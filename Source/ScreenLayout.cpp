#include "ScreenLayout.h"
#include "OledFont.h"

namespace ScreenLayout
{
//==============================================================================
juce::var Element::toVar() const
{
    auto* o = new juce::DynamicObject();
    o->setProperty ("k", kind == Kind::text ? "t" : kind == Kind::box ? "b" : "r");
    o->setProperty ("x", x);
    o->setProperty ("y", y);
    if (kind == Kind::text)
    {
        o->setProperty ("text", text);
        o->setProperty ("s", size);
        o->setProperty ("a", align);
        o->setProperty ("inv", inverted);
        o->setProperty ("fit", fit);
    }
    else
    {
        o->setProperty ("w", w);
        o->setProperty ("h", h);
    }
    return juce::var (o);
}

Element Element::fromVar (const juce::var& v)
{
    Element e;
    const auto k = v["k"].toString();
    e.kind = k == "b" ? Kind::box : k == "r" ? Kind::frame : Kind::text;
    e.x = (int) v.getProperty ("x", 0);
    e.y = (int) v.getProperty ("y", 0);
    e.text = v["text"].toString();
    e.size = juce::jlimit (1, 4, (int) v.getProperty ("s", 1));
    e.align = juce::jlimit (0, 2, (int) v.getProperty ("a", 1));
    e.inverted = (bool) v.getProperty ("inv", false);
    e.fit = (bool) v.getProperty ("fit", true);
    e.w = (int) v.getProperty ("w", 128);
    e.h = (int) v.getProperty ("h", 1);
    return e;
}

juce::var Layout::toVar() const
{
    juce::Array<juce::var> list;
    for (auto& e : elements) list.add (e.toVar());
    auto* o = new juce::DynamicObject();
    o->setProperty ("elements", list);
    o->setProperty ("bar", messageBar);
    return juce::var (o);
}

Layout Layout::fromVar (const juce::var& v)
{
    Layout l;
    if (auto* arr = v["elements"].getArray())
        for (auto& e : *arr)
            l.elements.push_back (Element::fromVar (e));
    l.messageBar = (bool) v.getProperty ("bar", true);
    return l;
}

Layout Layout::standard()
{
    Layout l;
    auto text = [&] (const char* t, int x, int y, int size, int align)
    {
        Element e;
        e.text = t; e.x = x; e.y = y; e.size = size; e.align = align;
        l.elements.push_back (e);
    };
    text ("{bank}", 64, 0, 1, 1);
    Element line;
    line.kind = Element::Kind::box;
    line.x = 0; line.y = 10; line.w = 128; line.h = 1;
    l.elements.push_back (line);
    text ("{preset}", 64, 22, 2, 1);
    text ("PRESET {num}  {bpm} BPM", 64, 56, 1, 1);
    return l;
}

//==============================================================================
juce::Rectangle<int> Item::bounds() const
{
    if (kind != 't') return { x, y, juce::jmax (1, w), juce::jmax (1, h) };
    const int tw = text.length() * 6 * size, th = 8 * size;
    const int left = align == 1 ? x - tw / 2 : align == 2 ? x - tw : x;
    auto r = juce::Rectangle<int> (left, y, juce::jmax (tw, 2), th);
    return inverted ? r.withX (left - 2).withY (y - 1).withWidth (tw + 3).withHeight (th + 1) : r;
}

namespace
{
    juce::String fillTokens (const juce::String& text, const std::function<juce::String (const juce::String&, int)>& lookup)
    {
        juce::String out;
        int i = 0;
        while (i < text.length())
        {
            const int open = text.indexOfChar (i, '{');
            if (open < 0) { out << text.substring (i); break; }
            const int close = text.indexOfChar (open, '}');
            if (close < 0) { out << text.substring (i); break; }
            out << text.substring (i, open);
            auto token = text.substring (open + 1, close).trim();
            auto name = token.upToFirstOccurrenceOf (" ", false, false).toLowerCase();
            const int number = token.fromFirstOccurrenceOf (" ", false, false).trim().getIntValue();
            out << lookup (name, number);
            i = close + 1;
        }
        // the font only has ASCII
        juce::String clean;
        for (auto c : out)
            clean += (c >= 32 && c < 127) ? (juce::juce_wchar) c : (juce::juce_wchar) '?';
        return clean;
    }
}

std::vector<Item> resolve (const Layout& layout, const std::function<juce::String (const juce::String&, int)>& lookup)
{
    std::vector<Item> items;
    for (size_t n = 0; n < layout.elements.size(); ++n)
    {
        auto& e = layout.elements[n];
        Item it;
        it.element = (int) n;
        it.x = e.x; it.y = e.y;
        if (e.kind != Element::Kind::text)
        {
            it.kind = e.kind == Element::Kind::box ? 'b' : 'r';
            it.w = e.w; it.h = e.h;
            items.push_back (it);
            continue;
        }
        it.kind = 't';
        it.text = fillTokens (e.text, lookup).trimEnd();
        it.size = e.size;
        it.align = e.align;
        it.inverted = e.inverted;

        if (e.fit)
        {
            // room from the anchor to the screen edge(s)
            const int room = e.align == 0 ? width - e.x : e.align == 2 ? e.x : 2 * juce::jmin (e.x, width - e.x);
            while (it.size > 1 && it.text.length() * 6 * it.size > room) --it.size;
            const int maxChars = juce::jmax (0, room / 6);
            if (it.text.length() > maxChars) it.text = it.text.substring (0, maxChars);
        }
        if (it.text.isNotEmpty()) items.push_back (it);
    }
    return items;
}

juce::var itemsToVar (const std::vector<Item>& items)
{
    juce::Array<juce::var> list;
    for (auto& it : items)
    {
        juce::Array<juce::var> e;
        e.add (juce::String::charToString ((juce::juce_wchar) it.kind));
        e.add (it.x);
        e.add (it.y);
        if (it.kind == 't') { e.add (it.size); e.add (it.align); e.add (it.inverted ? 1 : 0); e.add (it.text); }
        else                { e.add (it.w); e.add (it.h); }
        list.add (juce::var (e));
    }
    return list;
}

//==============================================================================
namespace
{
    void fill (juce::Image::BitmapData& px, int x, int y, int w, int h, bool white)
    {
        for (int yy = juce::jmax (0, y); yy < juce::jmin (height, y + h); ++yy)
            for (int xx = juce::jmax (0, x); xx < juce::jmin (width, x + w); ++xx)
                px.setPixelColour (xx, yy, white ? juce::Colours::white : juce::Colours::black);
    }
}

juce::Image render (const std::vector<Item>& items)
{
    juce::Image img (juce::Image::RGB, width, height, true);
    juce::Image::BitmapData px (img, juce::Image::BitmapData::readWrite);

    for (auto& it : items)
    {
        if (it.kind == 'b') { fill (px, it.x, it.y, it.w, it.h, true); continue; }
        if (it.kind == 'r')
        {
            fill (px, it.x, it.y, it.w, 1, true);
            fill (px, it.x, it.y + it.h - 1, it.w, 1, true);
            fill (px, it.x, it.y, 1, it.h, true);
            fill (px, it.x + it.w - 1, it.y, 1, it.h, true);
            continue;
        }
        const int s = it.size, tw = it.text.length() * 6 * s;
        int x = it.align == 1 ? it.x - tw / 2 : it.align == 2 ? it.x - tw : it.x;
        if (it.inverted) fill (px, x - 2, it.y - 1, tw + 3, 8 * s + 1, true);
        const bool ink = ! it.inverted;
        for (auto c : it.text)
        {
            const int code = (c >= 0 && c < 128) ? (int) c : '?';
            for (int col = 0; col < 5; ++col)
            {
                const unsigned char bits = OledFont::glyphs[code * 5 + col];
                for (int row = 0; row < 8; ++row)
                    if (bits & (1 << row))
                        fill (px, x + col * s, it.y + row * s, s, s, ink);
            }
            x += 6 * s;
        }
    }
    return img;
}

const std::vector<Token>& tokens()
{
    static const std::vector<Token> list {
        { "{preset}",  "Preset name" },
        { "{bank}",    "Bank name" },
        { "{num}",     "Preset number in the bank" },
        { "{next}",    "Name of the next preset" },
        { "{bpm}",     "Tempo (BPM)" },
        { "{beat}",    "Beat of the bar (1 2 3 4)" },
        { "{click}",   "CLICK while the metronome click is on" },
        { "{layer}",   "Layer (L1, L2...) when there is more than one" },
        { "{msg}",     "What a switch just did (for a moment)" },
        { "{fx 1}",    "Name of effect 1 in the rack (any number)" },
        { "{on 1}",    "ON / OFF of effect 1 in the rack" },
        { "{mute}",    "MUTED while PEQUOD is muted" },
        { "{name}",    "The controller's name" },
        { "{time}",    "Time of day (HH:MM)" },
    };
    return list;
}
}
