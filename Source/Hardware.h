#pragma once

// Drawing the rack as metal: powder-coated panels, the rails with their holes, keys that stand
// proud of the panel, lamps behind glass and the LED ladders of a meter. Everything is drawn
// with paths and gradients (or a texture made at twice the size), so it stays sharp at any scale.

namespace Pequod::hw
{
    /** A speckled powder-coat texture (made once). */
    inline const juce::Image& speckle()
    {
        static const juce::Image tile = []
        {
            juce::Image im (juce::Image::ARGB, 256, 256, true);
            juce::Graphics g (im);
            juce::Random rng (0x5eed);
            for (int i = 0; i < 2600; ++i)
            {
                const bool light = rng.nextBool();
                g.setColour ((light ? juce::Colours::white : juce::Colours::black).withAlpha (0.035f + rng.nextFloat() * 0.06f));
                const float s = 1.0f + rng.nextFloat() * 2.0f;
                g.fillEllipse (rng.nextFloat() * 256.0f, rng.nextFloat() * 256.0f, s, s);
            }
            return im;
        }();
        return tile;
    }

    /** A steel panel, powder-coated: a little lighter at the top (the room's light), speckled,
        with a bright top edge and a shadowed bottom one. */
    inline void panel (juce::Graphics& g, juce::Rectangle<float> r, juce::Colour base, float corner = 0.0f)
    {
        juce::Path p;
        if (corner > 0.0f) p.addRoundedRectangle (r, corner); else p.addRectangle (r);
        g.setGradientFill (juce::ColourGradient (base.brighter (0.10f), r.getX(), r.getY(), base.darker (0.18f), r.getX(), r.getBottom(), false));
        g.fillPath (p);
        g.saveState();
        g.reduceClipRegion (p);
        // (tiled at 1:1 on whole pixels: the fast way to fill with an image - scaling it costs a resample per pixel)
        g.setFillType (juce::FillType (speckle(), juce::AffineTransform::translation (std::floor (r.getX()), std::floor (r.getY()))));
        g.fillRect (r);
        g.restoreState();
        g.setColour (juce::Colours::white.withAlpha (0.10f));
        g.drawHorizontalLine ((int) r.getY(), r.getX() + corner, r.getRight() - corner);
        g.setColour (juce::Colours::black.withAlpha (0.55f));
        g.drawHorizontalLine ((int) r.getBottom() - 1, r.getX() + corner, r.getRight() - corner);
    }

    /** A slotted pan-head screw: chrome, with a shadow. */
    inline void screw (juce::Graphics& g, juce::Point<float> c, float rad, float angle = 0.6f)
    {
        g.setColour (juce::Colours::black.withAlpha (0.5f));
        g.fillEllipse (c.x - rad + 0.6f, c.y - rad + 1.0f, rad * 2.0f, rad * 2.0f);
        g.setGradientFill (juce::ColourGradient (juce::Colour (0xffd8d8d4), c.x - rad * 0.6f, c.y - rad * 0.7f,
                                                 juce::Colour (0xff4a4a48), c.x + rad * 0.7f, c.y + rad, true));
        g.fillEllipse (c.x - rad, c.y - rad, rad * 2.0f, rad * 2.0f);
        juce::Path slot;
        slot.addRectangle (-rad * 0.8f, -rad * 0.13f, rad * 1.6f, rad * 0.26f);
        g.setColour (juce::Colour (0xff202020));
        g.fillPath (slot, juce::AffineTransform::rotation (angle).translated (c));
    }

    /** One rail of the rack: plated steel, with its square holes in the EIA pattern
        (three to a unit, at 1/4", 7/8" and 1 1/2" from the top of the unit). */
    inline void rail (juce::Graphics& g, juce::Rectangle<float> r, float unit, float firstUnitY)
    {
        juce::ColourGradient steel (juce::Colour (0xff3c3d40), r.getX(), 0.0f, juce::Colour (0xff2a2b2d), r.getRight(), 0.0f, false);
        steel.addColour (0.35, juce::Colour (0xff57585c));
        g.setGradientFill (steel);
        g.fillRect (r);
        g.setColour (juce::Colours::black.withAlpha (0.6f));
        g.drawVerticalLine ((int) r.getRight() - 1, r.getY(), r.getBottom());
        const float hole = juce::jlimit (3.0f, 9.0f, r.getWidth() * 0.32f);
        int n = 1;
        for (float y = firstUnitY; y < r.getBottom(); y += unit, ++n)
        {
            for (float f : { 0.143f, 0.5f, 0.857f })
            {
                const float cy = y + unit * f;
                if (cy < r.getY() - hole || cy > r.getBottom() + hole) continue;
                auto h = juce::Rectangle<float> (hole, hole).withCentre ({ r.getCentreX(), cy });
                g.setColour (juce::Colour (0xff060606));
                g.fillRect (h);
                g.setColour (juce::Colours::white.withAlpha (0.12f));
                g.drawHorizontalLine ((int) h.getBottom(), h.getX(), h.getRight());
            }
            // the unit's number, stamped between its holes
            if (unit > 30.0f && y + unit * 0.3f > r.getY())
            {
                g.setColour (juce::Colours::white.withAlpha (0.22f));
                g.setFont (juce::Font (juce::FontOptions (juce::jlimit (7.0f, 10.0f, unit * 0.16f))));
                g.drawText (juce::String (n), juce::Rectangle<float> (r.getWidth(), unit * 0.2f).withCentre ({ r.getCentreX(), y + unit * 0.32f }),
                            juce::Justification::centred, false);
            }
        }
    }

    /** A push key standing proud of the panel; lit (for a latched key) is the cap glowing. */
    inline void key (juce::Graphics& g, juce::Rectangle<float> r, juce::Colour cap, bool down, bool lit, juce::Colour litColour)
    {
        // the hole in the panel
        g.setColour (juce::Colours::black.withAlpha (0.75f));
        g.fillRoundedRectangle (r.expanded (1.0f), 3.0f);
        auto body = r.reduced (1.0f).translated (0.0f, down ? 1.0f : 0.0f);
        if (! down) body = body.withTrimmedBottom (1.5f);
        auto c = lit ? litColour : cap;
        g.setGradientFill (juce::ColourGradient (c.brighter (lit ? 0.25f : 0.18f), body.getX(), body.getY(),
                                                 c.darker (lit ? 0.12f : 0.3f), body.getX(), body.getBottom(), false));
        g.fillRoundedRectangle (body, 2.5f);
        if (lit)
        {
            g.setGradientFill (juce::ColourGradient (juce::Colours::white.withAlpha (0.35f), body.getCentreX(), body.getCentreY(),
                                                     juce::Colours::transparentWhite, body.getX(), body.getY(), true));
            g.fillRoundedRectangle (body, 2.5f);
        }
        g.setColour (juce::Colours::white.withAlpha (down ? 0.05f : 0.16f));
        g.drawHorizontalLine ((int) body.getY() + 1, body.getX() + 2.0f, body.getRight() - 2.0f);
        g.setColour (juce::Colours::black.withAlpha (0.45f));
        g.drawRoundedRectangle (body, 2.5f, 0.8f);
    }

    /** A pilot lamp: a chrome bezel, the coloured glass dome, and its glow when on. */
    inline void lamp (juce::Graphics& g, juce::Point<float> c, float rad, juce::Colour colour, bool on)
    {
        if (on)
        {
            g.setGradientFill (juce::ColourGradient (colour.withAlpha (0.45f), c.x, c.y, colour.withAlpha (0.0f), c.x + rad * 2.6f, c.y, true));
            g.fillEllipse (c.x - rad * 2.6f, c.y - rad * 2.6f, rad * 5.2f, rad * 5.2f);
        }
        g.setGradientFill (juce::ColourGradient (juce::Colour (0xffcfcfcb), c.x, c.y - rad * 1.3f, juce::Colour (0xff3a3a38), c.x, c.y + rad * 1.3f, false));
        g.fillEllipse (c.x - rad * 1.3f, c.y - rad * 1.3f, rad * 2.6f, rad * 2.6f);
        const auto glass = on ? colour : colour.withMultipliedBrightness (0.28f).withMultipliedSaturation (0.7f);
        g.setGradientFill (juce::ColourGradient (glass.brighter (on ? 0.7f : 0.2f), c.x - rad * 0.3f, c.y - rad * 0.4f,
                                                 glass.darker (on ? 0.2f : 0.6f), c.x + rad, c.y + rad, true));
        g.fillEllipse (c.x - rad, c.y - rad, rad * 2.0f, rad * 2.0f);
        g.setColour (juce::Colours::white.withAlpha (on ? 0.7f : 0.25f));
        g.fillEllipse (c.x - rad * 0.55f, c.y - rad * 0.6f, rad * 0.5f, rad * 0.35f);
    }

    /** Text cut into the panel: a dark line under the letters. */
    inline void engraved (juce::Graphics& g, const juce::String& text, juce::Rectangle<float> r, juce::Colour colour, juce::Justification j)
    {
        g.setColour (juce::Colours::black.withAlpha (0.6f));
        g.drawText (text, r.translated (0.0f, 1.0f), j, true);
        g.setColour (colour);
        g.drawText (text, r, j, true);
    }

    /** A slot in the panel for a fader to run in. */
    inline void slot (juce::Graphics& g, juce::Rectangle<float> r)
    {
        g.setColour (juce::Colour (0xff050505));
        g.fillRoundedRectangle (r, r.getHeight() * 0.5f);
        g.setColour (juce::Colours::white.withAlpha (0.10f));
        g.drawHorizontalLine ((int) r.getBottom(), r.getX() + 2.0f, r.getRight() - 2.0f);
    }

    /** An LED ladder: `segments` lamps, lit up to `fraction`; the top ones amber, then red. */
    inline void ledLadder (juce::Graphics& g, juce::Rectangle<float> r, float fraction, int segments, juce::Colour low, bool vertical = true)
    {
        g.setColour (juce::Colour (0xff070707));
        g.fillRect (r);
        r = r.reduced (1.0f);
        const float len = vertical ? r.getHeight() : r.getWidth();
        const float step = len / (float) segments;
        for (int i = 0; i < segments; ++i)
        {
            const float at = (float) i / (float) segments;
            const auto colour = i >= segments - 1 ? juce::Colour (0xffe04030) : i >= segments - 3 ? juce::Colour (0xffe8b040) : low;
            const bool lit = fraction > at + 0.5f / (float) segments;
            auto seg = vertical ? juce::Rectangle<float> (r.getX(), r.getBottom() - step * (float) (i + 1), r.getWidth(), step)
                                : juce::Rectangle<float> (r.getX() + step * (float) i, r.getY(), step, r.getHeight());
            seg = vertical ? seg.withTrimmedTop (juce::jmin (1.0f, step * 0.3f)) : seg.withTrimmedRight (juce::jmin (1.0f, step * 0.3f));
            g.setColour (lit ? colour : colour.withMultipliedBrightness (0.18f));
            g.fillRect (seg);
        }
    }
    /** A display window let into the panel: a bezel, dark glass, and the glass's sheen on top
        (draw the readout between window() and glass()). */
    inline void window (juce::Graphics& g, juce::Rectangle<float> r, juce::Colour glassColour = juce::Colour (0xff060c08))
    {
        g.setColour (juce::Colours::black.withAlpha (0.7f));
        g.fillRoundedRectangle (r.expanded (2.0f), 5.0f);
        g.setGradientFill (juce::ColourGradient (juce::Colour (0xff101010), r.getX(), r.getY(), juce::Colour (0xff3a3a3a), r.getX(), r.getBottom(), false));
        g.fillRoundedRectangle (r.expanded (1.0f), 4.5f);
        g.setColour (glassColour);
        g.fillRoundedRectangle (r.reduced (1.5f), 3.5f);
    }
    inline void glass (juce::Graphics& g, juce::Rectangle<float> r)
    {
        auto in = r.reduced (1.5f);
        juce::Path sheen;
        sheen.startNewSubPath (in.getX(), in.getY());
        sheen.lineTo (in.getRight(), in.getY());
        sheen.lineTo (in.getRight(), in.getY() + in.getHeight() * 0.18f);
        sheen.quadraticTo (in.getCentreX(), in.getY() + in.getHeight() * 0.42f, in.getX(), in.getY() + in.getHeight() * 0.3f);
        sheen.closeSubPath();
        g.setColour (juce::Colours::white.withAlpha (0.05f));
        g.fillPath (sheen);
    }

    /** Lit characters on a display: their glow, then the characters (and the unlit segments behind them). */
    inline void lit (juce::Graphics& g, const juce::String& text, juce::Rectangle<float> r, juce::Colour colour, juce::Justification j,
                     const juce::String& ghost = {})
    {
        const auto box = r.toNearestInt();
        if (ghost.isNotEmpty())
        {
            g.setColour (colour.withAlpha (0.07f));
            g.drawFittedText (ghost, box, j, 1, 0.8f);
        }
        g.setColour (colour.withAlpha (0.18f));
        for (auto d : { juce::Point<int> (-1, 0), juce::Point<int> (1, 0), juce::Point<int> (0, -1), juce::Point<int> (0, 1) })
            g.drawFittedText (text, box + d, j, 1, 0.8f);
        g.setColour (colour);
        g.drawFittedText (text, box, j, 1, 0.8f);
    }
}
