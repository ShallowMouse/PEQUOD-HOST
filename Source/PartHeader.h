#pragma once

// The header every MELVILLE part shares (EMERSON, WHITMAN, THOREAU, IRVING, Hawthorne):
// the same height and layout, the same keys with the same line icons, the part's own colour.
// One copy of this file lives in each app's Source folder (keep them the same: MELVILLE
// builds them all into one program).
//
//   [ WORDMARK          status line ......................  [key] [key] [key] ]
//     sub-title         hint line                                              ___ part colour

#include <juce_gui_basics/juce_gui_basics.h>
#include <atomic>

namespace partui
{

constexpr int headerHeight = 58;      // the bar, its colour rule included
constexpr int keyWidth = 44, keyHeight = 36, keyGap = 6;

/** Each part's colour (as on MELVILLE's bar and patchbay). */
inline juce::Colour accent (const juce::String& part)
{
    if (part == "hawthorne") return juce::Colour (0xffe0a82e);   // mustard
    if (part == "emerson")   return juce::Colour (0xffe6d8b0);   // cream
    if (part == "whitman")   return juce::Colour (0xff7fb0d6);   // ink blue
    if (part == "thoreau")   return juce::Colour (0xff9fbf62);   // moss green
    if (part == "irving")    return juce::Colour (0xffa8443a);   // oxblood
    if (part == "twain")     return juce::Colour (0xffd9875a);
    if (part == "alcott")    return juce::Colour (0xffb27fc0);   // plum
    if (part == "poe")       return juce::Colour (0xff4ec3dc);   // lamp cyan
    return juce::Colour (0xff5fa8a0);                            // MELVILLE's verdigris
}

inline juce::Colour barColour()    { return juce::Colour (0xff141516); }
inline juce::Colour keyColour()    { return juce::Colour (0xff26282b); }
inline juce::Colour inkColour()    { return juce::Colour (0xffd8d2c4); }
inline juce::Colour dimColour()    { return juce::Colour (0xff8f8a80); }

//==============================================================================
/** MELVILLE's theme: the decade the header bars (and MELVILLE's own bar) are dressed for.
    STANDARD is the look they had before there was a choice. One setting for the whole
    program (MELVILLE keeps it), read whenever a header is drawn. The bars stay dark in every
    decade, so the parts' own status lines read on all of them. */
enum class Era { standard = 0, forties, fifties, sixties, seventies, eighties, nineties, noughties, tens, today };
constexpr int numEras = 10;

inline std::atomic<int>& eraState() { static std::atomic<int> e { 0 }; return e; }
inline Era era()                     { return (Era) juce::jlimit (0, numEras - 1, eraState().load()); }
inline void setEra (Era e)           { eraState() = (int) e; }
inline juce::String eraName (Era e)
{
    static const char* names[] = { "Standard", "1940s", "1950s", "1960s", "1970s", "1980s", "1990s", "2000s", "2010s", "Today" };
    return names[juce::jlimit (0, numEras - 1, (int) e)];
}

/** A typeface every Windows has, with the one Linux has for it. */
inline juce::Font eraTypeface (const char* windows, const char* linuxName, float size, int style)
{
   #if JUCE_WINDOWS || JUCE_MAC
    juce::ignoreUnused (linuxName);
    return juce::Font (juce::FontOptions (windows, size, style));
   #else
    juce::ignoreUnused (windows);
    return juce::Font (juce::FontOptions (linuxName, size, style));
   #endif
}

/** A decade's lettering for names (the parts' wordmarks, MELVILLE's own). */
inline juce::Font eraNameFont (Era e, float size)
{
    switch (e)
    {
        case Era::forties:   return eraTypeface ("Georgia", "DejaVu Serif", size * 0.92f, juce::Font::bold);                       // engraved serif plates
        case Era::fifties:   return eraTypeface ("Georgia", "DejaVu Serif", size * 0.92f, juce::Font::bold | juce::Font::italic);  // chrome script
        case Era::sixties:   return eraTypeface ("Arial", "Liberation Sans", size * 0.8f, juce::Font::bold).withExtraKerningFactor (0.14f);   // the Swiss grotesque
        case Era::seventies: return eraTypeface ("Arial Black", "DejaVu Sans", size * 0.8f, juce::Font::bold).withExtraKerningFactor (-0.02f); // heavy
        case Era::eighties:  return eraTypeface ("Arial", "Liberation Sans", size * 0.86f, juce::Font::bold | juce::Font::italic).withExtraKerningFactor (0.03f);
        case Era::nineties:  return eraTypeface ("Verdana", "DejaVu Sans", size * 0.74f, juce::Font::bold | juce::Font::italic);
        case Era::noughties: return eraTypeface ("Tahoma", "DejaVu Sans", size * 0.78f, juce::Font::bold);
        case Era::tens:      return eraTypeface ("Segoe UI Light", "Liberation Sans", size * 0.86f, juce::Font::plain).withExtraKerningFactor (0.06f);
        case Era::today:     return eraTypeface ("Segoe UI Semibold", "Liberation Sans", size * 0.84f, juce::Font::bold);
        case Era::standard:  break;
    }
    return juce::Font (juce::FontOptions (size, juce::Font::bold));
}

/** The panel behind a bar in a decade's dress (a header, or MELVILLE's own bar). */
inline void drawEraPanel (juce::Graphics& g, juce::Rectangle<float> r, Era e)
{
    const float x = r.getX(), top = r.getY(), bot = r.getBottom();
    auto grad = [&] (juce::Colour a, juce::Colour b) { g.setGradientFill (juce::ColourGradient (a, x, top, b, x, bot, false)); g.fillRect (r); };
    switch (e)
    {
        case Era::forties:
        {
            // brown Bakelite, polished
            grad (juce::Colour (0xff3a261a), juce::Colour (0xff1f130c));
            g.setColour (juce::Colours::white.withAlpha (0.07f));
            g.fillRect (r.withHeight (r.getHeight() * 0.35f));
            break;
        }
        case Era::fifties:
        {
            // grey-green hammertone, the broadcast desks' enamel
            grad (juce::Colour (0xff47534a), juce::Colour (0xff2f3832));
            juce::Random rnd (1953);
            for (int i = 0; i < (int) (r.getWidth() * r.getHeight() / 90.0f); ++i)
            {
                g.setColour ((rnd.nextBool() ? juce::Colours::white : juce::Colours::black).withAlpha (0.05f + 0.05f * rnd.nextFloat()));
                const float d = 1.0f + 2.5f * rnd.nextFloat();
                g.fillEllipse (x + rnd.nextFloat() * r.getWidth(), top + rnd.nextFloat() * r.getHeight(), d, d);
            }
            break;
        }
        case Era::sixties:
        {
            // dark brushed aluminium
            grad (juce::Colour (0xff4b4e52), juce::Colour (0xff33363a));
            juce::Random rnd (1966);
            for (float y = top; y < bot; y += 1.0f)
            {
                g.setColour ((rnd.nextBool() ? juce::Colours::white : juce::Colours::black).withAlpha (0.03f + 0.04f * rnd.nextFloat()));
                g.drawHorizontalLine ((int) y, x, r.getRight());
            }
            break;
        }
        case Era::seventies:
        {
            // black anodised, walnut at the ends
            grad (juce::Colour (0xff1c1b1a), juce::Colour (0xff0e0d0d));
            for (auto end : { r.withWidth (10.0f), r.withLeft (r.getRight() - 10.0f) })
            {
                g.setGradientFill (juce::ColourGradient (juce::Colour (0xff6a4024), end.getX(), 0.0f, juce::Colour (0xff3b2213), end.getRight(), 0.0f, false));
                g.fillRect (end);
            }
            break;
        }
        case Era::eighties:
        {
            // charcoal, with the band of four colours along the foot
            grad (juce::Colour (0xff2e2e2f), juce::Colour (0xff202021));
            const float w = r.getWidth() / 4.0f;
            const juce::Colour band[] = { juce::Colour (0xffd8432f), juce::Colour (0xffea7f2a), juce::Colour (0xffe9c53b), juce::Colour (0xffe7e0cc) };
            for (int i = 0; i < 4; ++i) { g.setColour (band[i]); g.fillRect (juce::Rectangle<float> (x + w * (float) i, bot - 6.0f, w, 3.0f)); }
            break;
        }
        case Era::nineties:
        {
            // a 1U rack unit: blue-grey, with the ears' slots
            grad (juce::Colour (0xff2d3240), juce::Colour (0xff1c1f29));
            g.setColour (juce::Colours::white.withAlpha (0.08f));
            g.drawHorizontalLine ((int) top + 1, x, r.getRight());
            for (auto cx : { x + 10.0f, r.getRight() - 10.0f })
            {
                g.setColour (juce::Colour (0xff0b0c10));
                g.fillRoundedRectangle (juce::Rectangle<float> (5.0f, 12.0f).withCentre ({ cx, r.getCentreY() }), 2.5f);
            }
            break;
        }
        case Era::noughties:
        {
            // dark glass with a gloss across the top half
            grad (juce::Colour (0xff2b3138), juce::Colour (0xff0f1215));
            g.setGradientFill (juce::ColourGradient (juce::Colours::white.withAlpha (0.16f), x, top, juce::Colours::white.withAlpha (0.03f), x, r.getCentreY(), false));
            g.fillRect (r.withHeight (r.getHeight() * 0.48f));
            break;
        }
        case Era::tens:
            g.setColour (juce::Colour (0xff1f2023));      // flat
            g.fillRect (r);
            break;
        case Era::today:
            g.setColour (juce::Colour (0xff17181b));
            g.fillRect (r);
            g.setColour (juce::Colours::white.withAlpha (0.05f));
            g.drawHorizontalLine ((int) top, x, r.getRight());
            break;
        case Era::standard:
            g.setGradientFill (juce::ColourGradient (juce::Colour (0xff1b1c1e), 0.0f, top, barColour(), 0.0f, bot, false));
            g.fillRect (r);
            break;
    }
}

/** The bar behind a header: its decade's panel, with the part's colour as a rule along the bottom. */
inline void drawBar (juce::Graphics& g, juce::Rectangle<int> bar, juce::Colour partColour)
{
    const auto e = era();
    drawEraPanel (g, bar.toFloat(), e);
    switch (e)
    {
        case Era::forties:      // a brass line over the part's
            g.setColour (juce::Colour (0xffc9a04a));
            g.fillRect (bar.withHeight (1).withY (bar.getBottom() - 4));
            break;
        case Era::fifties:      // chrome
            g.setColour (juce::Colour (0xffd9dcdc));
            g.fillRect (bar.withHeight (1).withY (bar.getBottom() - 4));
            break;
        case Era::tens:         // flat: a heavier rule
            g.setColour (partColour);
            g.fillRect (bar.removeFromBottom (4));
            return;
        default: break;
    }
    g.setColour (partColour);
    g.fillRect (bar.removeFromBottom (2));
}

enum class Icon { none, settings, folder, open, save, reset, update, help, fullScreen, exitFullScreen, keys, midi, library,
                  sheet, routing, network, tempo, timecode, layout, machine, input, plus, power, tuner, list };

/** Line icons, all drawn the same way (Hawthorne's style): a stroke of about 1/12 of the size. */
inline void drawIcon (juce::Graphics& g, Icon icon, juce::Rectangle<float> area, juce::Colour c)
{
    if (icon == Icon::none) return;
    const float s = juce::jmin (area.getWidth(), area.getHeight());
    auto b = area.withSizeKeepingCentre (s, s);
    auto P = [&] (float fx, float fy) { return juce::Point<float> (b.getX() + fx * s, b.getY() + fy * s); };
    const float stroke = juce::jmax (1.2f, s * 0.085f);
    const juce::PathStrokeType st (stroke, juce::PathStrokeType::curved, juce::PathStrokeType::rounded);
    g.setColour (c);
    juce::Path p;
    auto line = [&] (float x0, float y0, float x1, float y1) { g.drawLine ({ P (x0, y0), P (x1, y1) }, stroke); };
    auto box  = [&] (float x0, float y0, float x1, float y1) { g.drawRoundedRectangle (juce::Rectangle<float> (P (x0, y0), P (x1, y1)), s * 0.05f, stroke); };
    auto ring = [&] (float cx, float cy, float r) { g.drawEllipse (juce::Rectangle<float> (r * 2 * s, r * 2 * s).withCentre (P (cx, cy)), stroke); };
    auto dot  = [&] (float cx, float cy, float r) { g.fillEllipse (juce::Rectangle<float> (r * 2 * s, r * 2 * s).withCentre (P (cx, cy))); };
    auto arrow = [&] (float x, float y0, float y1)          // a vertical arrow pointing to y1
    {
        line (x, y0, x, y1);
        const float d = y1 > y0 ? -0.14f : 0.14f;
        line (x - 0.14f, y1 + d, x, y1);
        line (x + 0.14f, y1 + d, x, y1);
    };

    switch (icon)
    {
        case Icon::settings:
        {
            // a cog
            ring (0.5f, 0.5f, 0.13f);
            for (int k = 0; k < 8; ++k)
            {
                const float a = juce::MathConstants<float>::twoPi * (float) k / 8.0f;
                line (0.5f + 0.25f * std::cos (a), 0.5f + 0.25f * std::sin (a), 0.5f + 0.36f * std::cos (a), 0.5f + 0.36f * std::sin (a));
            }
            ring (0.5f, 0.5f, 0.25f);
            break;
        }
        case Icon::folder:
        case Icon::open:
            p.startNewSubPath (P (0.12f, 0.28f)); p.lineTo (P (0.40f, 0.28f)); p.lineTo (P (0.47f, 0.36f)); p.lineTo (P (0.88f, 0.36f));
            p.lineTo (P (0.88f, 0.78f)); p.lineTo (P (0.12f, 0.78f)); p.closeSubPath();
            g.strokePath (p, st);
            if (icon == Icon::open) arrow (0.5f, 0.70f, 0.46f);
            break;
        case Icon::save:
            box (0.18f, 0.16f, 0.82f, 0.84f);
            line (0.32f, 0.16f, 0.32f, 0.36f); line (0.32f, 0.36f, 0.66f, 0.36f); line (0.66f, 0.36f, 0.66f, 0.16f);
            box (0.32f, 0.56f, 0.68f, 0.84f);
            break;
        case Icon::reset:
            p.addCentredArc (P (0.5f, 0.52f).x, P (0.5f, 0.52f).y, s * 0.28f, s * 0.28f, 0.0f, -2.2f, 2.6f, true);
            g.strokePath (p, st);
            line (0.25f, 0.24f, 0.25f, 0.40f); line (0.25f, 0.40f, 0.41f, 0.40f);
            break;
        case Icon::update:
            arrow (0.5f, 0.16f, 0.66f);
            line (0.18f, 0.82f, 0.82f, 0.82f);
            break;
        case Icon::help:
            ring (0.5f, 0.5f, 0.36f);
            p.startNewSubPath (P (0.38f, 0.40f));
            p.quadraticTo (P (0.40f, 0.26f), P (0.52f, 0.27f));
            p.quadraticTo (P (0.64f, 0.29f), P (0.62f, 0.41f));
            p.quadraticTo (P (0.60f, 0.49f), P (0.50f, 0.55f));
            p.lineTo (P (0.50f, 0.60f));
            g.strokePath (p, st);
            dot (0.5f, 0.72f, 0.045f);
            break;
        case Icon::fullScreen:
            for (auto [x, y, dx, dy] : { std::tuple (0.16f, 0.16f, 1.f, 1.f), std::tuple (0.84f, 0.16f, -1.f, 1.f),
                                         std::tuple (0.16f, 0.84f, 1.f, -1.f), std::tuple (0.84f, 0.84f, -1.f, -1.f) })
            { line (x, y, x + 0.22f * dx, y); line (x, y, x, y + 0.22f * dy); }
            break;
        case Icon::exitFullScreen:
            for (auto [x, y, dx, dy] : { std::tuple (0.38f, 0.38f, -1.f, -1.f), std::tuple (0.62f, 0.38f, 1.f, -1.f),
                                         std::tuple (0.38f, 0.62f, -1.f, 1.f), std::tuple (0.62f, 0.62f, 1.f, 1.f) })
            { line (x, y, x + 0.22f * dx, y); line (x, y, x, y + 0.22f * dy); }
            break;
        case Icon::keys:
            box (0.12f, 0.24f, 0.88f, 0.78f);
            for (float x : { 0.31f, 0.50f, 0.69f }) line (x, 0.24f, x, 0.78f);
            for (float x : { 0.31f, 0.69f }) g.fillRect (juce::Rectangle<float> (P (x - 0.06f, 0.24f), P (x + 0.06f, 0.52f)));
            break;
        case Icon::midi:
            ring (0.5f, 0.5f, 0.36f);
            for (float a : { -2.2f, -1.1f, 0.0f, 1.1f, 2.2f }) dot (0.5f + 0.21f * std::sin (a), 0.5f - 0.21f * std::cos (a) + 0.06f, 0.04f);
            g.fillRect (juce::Rectangle<float> (P (0.45f, 0.82f), P (0.55f, 0.88f)));
            break;
        case Icon::library:
            for (float x : { 0.18f, 0.36f, 0.54f }) box (x, 0.22f, x + 0.13f, 0.80f);
            p.startNewSubPath (P (0.70f, 0.26f)); p.lineTo (P (0.84f, 0.22f)); p.lineTo (P (0.88f, 0.78f)); p.lineTo (P (0.74f, 0.80f)); p.closeSubPath();
            g.strokePath (p, st);
            break;
        case Icon::sheet:
        case Icon::list:
            box (0.22f, 0.14f, 0.78f, 0.86f);
            for (float y : { 0.34f, 0.50f, 0.66f }) line (0.34f, y, 0.66f, y);
            break;
        case Icon::routing:
            for (float y : { 0.3f, 0.7f }) { dot (0.2f, y, 0.06f); dot (0.8f, y, 0.06f); }
            line (0.2f, 0.3f, 0.8f, 0.7f); line (0.2f, 0.7f, 0.8f, 0.3f);
            break;
        case Icon::network:
            box (0.36f, 0.14f, 0.64f, 0.34f); box (0.12f, 0.66f, 0.40f, 0.86f); box (0.60f, 0.66f, 0.88f, 0.86f);
            line (0.5f, 0.34f, 0.5f, 0.50f); line (0.26f, 0.50f, 0.74f, 0.50f); line (0.26f, 0.50f, 0.26f, 0.66f); line (0.74f, 0.50f, 0.74f, 0.66f);
            break;
        case Icon::tempo:
            p.startNewSubPath (P (0.38f, 0.14f)); p.lineTo (P (0.62f, 0.14f)); p.lineTo (P (0.78f, 0.86f)); p.lineTo (P (0.22f, 0.86f)); p.closeSubPath();
            g.strokePath (p, st);
            line (0.5f, 0.70f, 0.70f, 0.30f);
            break;
        case Icon::timecode:
            ring (0.5f, 0.52f, 0.34f);
            line (0.5f, 0.52f, 0.5f, 0.30f); line (0.5f, 0.52f, 0.66f, 0.60f);
            break;
        case Icon::layout:
            box (0.14f, 0.18f, 0.86f, 0.82f);
            line (0.14f, 0.40f, 0.86f, 0.40f); line (0.46f, 0.40f, 0.46f, 0.82f);
            break;
        case Icon::machine:
            ring (0.30f, 0.40f, 0.17f); ring (0.70f, 0.40f, 0.17f);
            line (0.18f, 0.80f, 0.82f, 0.80f); line (0.30f, 0.57f, 0.42f, 0.80f); line (0.70f, 0.57f, 0.58f, 0.80f);
            break;
        case Icon::input:
            line (0.12f, 0.5f, 0.56f, 0.5f);
            line (0.42f, 0.36f, 0.56f, 0.5f); line (0.42f, 0.64f, 0.56f, 0.5f);
            line (0.80f, 0.22f, 0.80f, 0.78f); line (0.66f, 0.22f, 0.80f, 0.22f); line (0.66f, 0.78f, 0.80f, 0.78f);
            break;
        case Icon::plus:
            line (0.5f, 0.2f, 0.5f, 0.8f); line (0.2f, 0.5f, 0.8f, 0.5f);
            break;
        case Icon::power:
            p.addCentredArc (P (0.5f, 0.54f).x, P (0.5f, 0.54f).y, s * 0.30f, s * 0.30f, 0.0f, 0.55f, juce::MathConstants<float>::twoPi - 0.55f, true);
            g.strokePath (p, st);
            line (0.5f, 0.14f, 0.5f, 0.50f);
            break;
        case Icon::tuner:
            p.addCentredArc (P (0.5f, 0.74f).x, P (0.5f, 0.74f).y, s * 0.40f, s * 0.40f, 0.0f, -1.2f, 1.2f, true);
            g.strokePath (p, st);
            line (0.5f, 0.74f, 0.62f, 0.34f);
            break;
        case Icon::none: break;
    }
}

/** A header key's body in its decade's dress; returns the cap, for its icon. */
inline juce::Rectangle<float> drawKey (juce::Graphics& g, juce::Rectangle<float> r, bool on, bool over, bool down, juce::Colour partColour)
{
    const auto e = era();
    auto cap = r.reduced (1.5f).translated (0.0f, down ? 1.0f : 0.0f);
    auto gradFill = [&] (juce::Colour base, float corner, float hi = 0.08f, float lo = 0.25f)
    {
        g.setGradientFill (juce::ColourGradient (base.brighter (over ? hi + 0.1f : hi), 0.0f, cap.getY(), base.darker (lo), 0.0f, cap.getBottom(), false));
        g.fillRoundedRectangle (cap, corner);
    };
    switch (e)
    {
        case Era::forties:      // a Bakelite button in a brass bezel
            g.setColour (juce::Colour (0xffa9843c));
            g.fillRoundedRectangle (r, 7.0f);
            gradFill (on ? partColour.withMultipliedBrightness (0.5f) : juce::Colour (0xff3b2619), 6.0f, 0.12f, 0.3f);
            g.setColour (juce::Colours::white.withAlpha (0.10f));
            g.fillRoundedRectangle (cap.withHeight (cap.getHeight() * 0.4f).reduced (3.0f, 1.0f), 4.0f);
            return cap;
        case Era::fifties:      // black, in a chrome surround
            g.setGradientFill (juce::ColourGradient (juce::Colour (0xffeef0f0), 0.0f, r.getY(), juce::Colour (0xff7d8283), 0.0f, r.getBottom(), false));
            g.fillRoundedRectangle (r, 4.0f);
            gradFill (on ? partColour.withMultipliedBrightness (0.5f) : juce::Colour (0xff161616), 3.0f);
            return cap;
        case Era::sixties:      // a square grey pushbutton
            g.setColour (juce::Colour (0xff161718));
            g.fillRect (r);
            gradFill (on ? partColour.withMultipliedBrightness (0.85f) : juce::Colour (0xffb9bab6), 1.0f, 0.05f, 0.15f);
            g.setColour (juce::Colours::white.withAlpha (0.35f));
            g.drawHorizontalLine ((int) cap.getY() + 1, cap.getX() + 1.0f, cap.getRight() - 1.0f);
            return cap;
        case Era::seventies:    // black, the cap lit in its colour when it's on
            g.setColour (juce::Colour (0xff050505));
            g.fillRoundedRectangle (r, 3.0f);
            gradFill (juce::Colour (0xff222120), 2.0f);
            g.setColour (on ? partColour : juce::Colour (0xff3a3836));
            g.fillRect (cap.removeFromBottom (4.0f).reduced (4.0f, 0.5f));
            if (on)
            {
                g.setColour (partColour.withAlpha (0.25f));
                g.fillRoundedRectangle (r.expanded (1.0f).withTop (cap.getBottom() - 6.0f), 3.0f);
            }
            return cap;
        case Era::eighties:     // a plastic key, the part's colour when on
        {
            g.setColour (juce::Colours::black.withAlpha (0.6f));
            g.fillRoundedRectangle (r.translated (0.0f, down ? 1.0f : 2.0f), 2.5f);
            gradFill (on ? partColour : juce::Colour (0xff8f8e8a), 2.5f, 0.1f, 0.15f);
            g.setColour (juce::Colours::white.withAlpha (0.25f));
            g.drawHorizontalLine ((int) cap.getY() + 1, cap.getX() + 2.0f, cap.getRight() - 2.0f);
            return cap;
        }
        case Era::nineties:     // soft rubber, with an LED over it
            g.setColour (juce::Colour (0xff0e0f14));
            g.fillRoundedRectangle (r, 7.0f);
            gradFill (juce::Colour (0xff3a3f4c), 6.0f, 0.06f, 0.2f);
            g.setColour (on ? juce::Colour (0xff5ef2d0) : juce::Colour (0xff1d3a35));
            g.fillEllipse (juce::Rectangle<float> (4.0f, 4.0f).withCentre ({ cap.getRight() - 6.0f, cap.getY() + 6.0f }));
            return cap;
        case Era::noughties:    // a glossy pill
        {
            const float rad = cap.getHeight() * 0.5f;
            g.setColour (juce::Colours::black.withAlpha (0.7f));
            g.fillRoundedRectangle (r, rad + 1.0f);
            gradFill (on ? partColour.withMultipliedBrightness (0.75f) : juce::Colour (0xff3d4550), rad, 0.15f, 0.35f);
            g.setGradientFill (juce::ColourGradient (juce::Colours::white.withAlpha (0.38f), 0.0f, cap.getY(), juce::Colours::white.withAlpha (0.05f), 0.0f, cap.getCentreY(), false));
            g.fillRoundedRectangle (cap.withHeight (cap.getHeight() * 0.5f).reduced (2.0f, 1.0f), rad * 0.8f);
            return cap;
        }
        case Era::tens:         // flat
            g.setColour (on ? partColour : over ? juce::Colour (0xff33353a) : juce::Colour (0xff2a2c30));
            g.fillRect (cap);
            return cap;
        case Era::today:        // a soft rounded key
            g.setColour (on ? partColour.withAlpha (0.28f) : juce::Colours::white.withAlpha (over ? 0.10f : 0.055f));
            g.fillRoundedRectangle (cap, 8.0f);
            if (on) { g.setColour (partColour.withAlpha (0.7f)); g.drawRoundedRectangle (cap, 8.0f, 1.0f); }
            return cap;
        case Era::standard:
            break;
    }
    g.setColour (juce::Colour (0xff0d0e0f));
    g.fillRoundedRectangle (r, 5.0f);
    const auto base = on ? partColour.withMultipliedBrightness (0.45f) : keyColour();
    g.setGradientFill (juce::ColourGradient (base.brighter (over ? 0.18f : 0.08f), 0.0f, cap.getY(), base.darker (0.25f), 0.0f, cap.getBottom(), false));
    g.fillRoundedRectangle (cap, 4.0f);
    g.setColour (juce::Colours::white.withAlpha (0.06f));
    g.drawRoundedRectangle (cap, 4.0f, 1.0f);
    return cap;
}

/** The ink a key's icon is drawn in. */
inline juce::Colour keyInk (bool on, bool over, bool enabled, juce::Colour partColour)
{
    const float a = enabled ? 1.0f : 0.35f;
    switch (era())
    {
        case Era::forties:   return (on ? juce::Colours::white : juce::Colour (0xffeadfc4)).withMultipliedAlpha (a);    // ivory
        case Era::sixties:   return (on && partColour.getPerceivedBrightness() < 0.6f ? juce::Colours::white : juce::Colour (0xff1c1c1c)).withMultipliedAlpha (a);
        case Era::eighties:  return (on && partColour.getPerceivedBrightness() < 0.6f ? juce::Colours::white : juce::Colour (0xff1a1a1a)).withMultipliedAlpha (a);
        case Era::seventies: return (on ? partColour.brighter (0.3f) : juce::Colour (0xffe8d9b5)).withMultipliedAlpha (a);    // amber-white legends
        case Era::tens:      return (on ? (partColour.getPerceivedBrightness() > 0.6f ? juce::Colour (0xff141414) : juce::Colours::white)
                                        : over ? juce::Colours::white : juce::Colour (0xffb8bcc2)).withMultipliedAlpha (a);
        default: break;
    }
    return (on ? juce::Colours::white : over ? partColour.brighter (0.2f) : inkColour()).withMultipliedAlpha (a);
}

/** A header key: a dark key with a line icon (its name is the tooltip); lit, it takes the part's colour. */
class Key : public juce::TextButton
{
public:
    Key (const juce::String& name, Icon i, juce::Colour partColour = accent ({})) : juce::TextButton (name), icon (i), colour (partColour)
    {
        setTooltip (name);
        setWantsKeyboardFocus (false);
    }

    void setIcon (Icon i)            { if (i != icon) { icon = i; repaint(); } }
    void setPartColour (juce::Colour c) { colour = c; repaint(); }

    void paintButton (juce::Graphics& g, bool over, bool down) override
    {
        const bool on = getToggleState();
        const auto cap = drawKey (g, getLocalBounds().toFloat().reduced (1.0f), on, over, down, colour);
        const auto ink = keyInk (on, over, isEnabled(), colour);
        const float sz = juce::jmin (22.0f, cap.getWidth() - 12.0f, cap.getHeight() - 10.0f);
        if (icon != Icon::none)
            drawIcon (g, icon, cap.withSizeKeepingCentre (sz, sz), ink);
        else
        {
            g.setColour (ink);
            g.setFont (juce::Font (juce::FontOptions (13.0f, juce::Font::bold)));
            g.drawFittedText (getButtonText(), cap.toNearestInt().reduced (6, 2), juce::Justification::centred, 1);
        }
    }

private:
    Icon icon;
    juce::Colour colour;
};

/** Lays out header keys from the right of a bar (skipping hidden ones); returns what's left. */
inline juce::Rectangle<int> placeKeys (juce::Rectangle<int> bar, std::initializer_list<juce::Component*> keys, int width = keyWidth)
{
    auto row = bar.withSizeKeepingCentre (bar.getWidth(), keyHeight);
    for (auto* k : keys)
    {
        if (k == nullptr || ! k->isVisible()) continue;
        k->setBounds (row.removeFromRight (k->getProperties().contains ("wide") ? (int) k->getProperties()["wide"] : width));
        row.removeFromRight (keyGap);
    }
    return bar.withRight (row.getRight());
}

/** The wordmark: the part's name in its colour, and a small dim sub-title under it. */
inline void drawWordmark (juce::Graphics& g, juce::Rectangle<int> area, const juce::String& name, const juce::String& sub,
                          juce::Colour partColour, juce::Font nameFont)
{
    auto a = area.withSizeKeepingCentre (area.getWidth(), 42);
    if (era() != Era::standard) nameFont = eraNameFont (era(), nameFont.getHeight());    // (the decade's lettering)
    g.setColour (partColour);
    g.setFont (nameFont);
    g.drawFittedText (name, a.removeFromTop (28), juce::Justification::bottomLeft, 1, 0.75f);
    g.setColour (dimColour());
    g.setFont (juce::Font (juce::FontOptions (10.5f, juce::Font::bold)));
    g.drawText (sub, a, juce::Justification::topLeft, false);
}

} // namespace partui
