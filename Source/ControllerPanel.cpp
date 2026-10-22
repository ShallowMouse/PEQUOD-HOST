#include "ControllerPanel.h"
#include "PluginColours.h"

namespace
{
    const juce::uint32 swatchColours[] = { 0xE0A82E, 0xF44336, 0xFF7A1A, 0xFFD600, 0x4CAF50,
                                           0x00BCD4, 0x2196F3, 0x9C27B0, 0xE91E63, 0xFFFFFF };

    juce::String hex (juce::Colour c) { return "#" + c.toDisplayString (false).toUpperCase(); }

    void styleCombo (juce::ComboBox& b)
    {
        b.setColour (juce::ComboBox::backgroundColourId, Pequod::Colours::well);
        b.setColour (juce::ComboBox::outlineColourId, Pequod::Colours::line);
    }

    void styleEditor (juce::TextEditor& e)
    {
        e.setColour (juce::TextEditor::backgroundColourId, Pequod::Colours::well);
        e.setColour (juce::TextEditor::outlineColourId, Pequod::Colours::line);
        e.setColour (juce::TextEditor::focusedOutlineColourId, Pequod::Colours::mustard);
        e.setFont (Pequod::body (14.0f));
        e.setIndents (8, 6);
    }

    void styleSlider (juce::Slider& s)
    {
        s.setSliderStyle (juce::Slider::LinearHorizontal);
        s.setTextBoxStyle (juce::Slider::TextBoxRight, false, 56, 22);
        s.setColour (juce::Slider::trackColourId, Pequod::Colours::mustard);
        s.setColour (juce::Slider::backgroundColourId, Pequod::Colours::well);
        s.setColour (juce::Slider::textBoxOutlineColourId, Pequod::Colours::line);
    }

    /** Lays out (label, component) rows from the top. */
    struct Form
    {
        struct Row { juce::String label; juce::Array<juce::Component*> comps; int h = 28; bool heading = false; int labelW = -1; bool newColumn = false; };
        std::vector<Row> rows;
        std::vector<std::pair<juce::String, juce::Rectangle<int>>> labels, headings;
        int labelWidth = 120;

        void clear() { rows.clear(); }
        void heading (const juce::String& t) { Row r; r.label = t; r.heading = true; r.h = 34; rows.push_back (r); }
        void row (const juce::String& l, std::initializer_list<juce::Component*> c, int h = 28)
        {
            Row r; r.label = l; r.h = h;
            for (auto* x : c) r.comps.add (x);
            rows.push_back (r);
        }
        void wide (std::initializer_list<juce::Component*> c, int h = 28) { Row r; r.labelW = 0; r.h = h; for (auto* x : c) r.comps.add (x); rows.push_back (r); }
        void gap (int h) { Row r; r.h = h; rows.push_back (r); }
        /** What follows goes in the next column (when laid out side by side). */
        void column() { if (rows.empty() || rows.back().newColumn) return; Row r; r.newColumn = true; r.h = 0; rows.push_back (r); }

        int layout (juce::Rectangle<int> full)
        {
            labels.clear(); headings.clear();
            int columns = 1;
            for (auto& r : rows) if (r.newColumn) ++columns;
            const int colGap = 28;
            if (columns > 1 && (full.getWidth() - colGap * (columns - 1)) / columns < 340) columns = 1;   // too narrow: one column
            const bool stack = columns == 1;
            const int colW = (full.getWidth() - colGap * (columns - 1)) / columns;
            auto area = full.withWidth (colW);
            int y = area.getY(), tallest = 0;
            for (auto& r : rows)
            {
                if (r.newColumn)
                {
                    if (stack) continue;
                    tallest = juce::jmax (tallest, y - full.getY());
                    area = area.translated (colW + colGap, 0);
                    y = full.getY();
                    continue;
                }
                juce::Rectangle<int> line (area.getX(), y, area.getWidth(), r.h);
                y += r.h + 6;
                if (r.heading) { headings.push_back ({ r.label, line.withTrimmedTop (10) }); continue; }
                const int lw = r.labelW >= 0 ? r.labelW : (r.label.isEmpty() && r.comps.isEmpty() ? 0 : labelWidth);
                if (lw > 0) labels.push_back ({ r.label, line.removeFromLeft (lw) });
                if (r.comps.isEmpty()) continue;
                if (r.comps[0]->getProperties().contains ("w"))      // fixed widths: left to right
                {
                    for (auto* c : r.comps)
                    {
                        c->setBounds (line.removeFromLeft ((int) c->getProperties()["w"]));
                        line.removeFromLeft (6);
                    }
                    continue;
                }
                // the first component takes what the others leave (others keep their own widths)
                for (int i = r.comps.size() - 1; i > 0; --i)
                {
                    auto* c = r.comps[i];
                    const int w = (int) c->getProperties().getWithDefault ("w", 90);
                    c->setBounds (line.removeFromRight (w));
                    line.removeFromRight (6);
                }
                r.comps[0]->setBounds (line);
            }
            return juce::jmax (tallest, y - full.getY());
        }

        void paint (juce::Graphics& g) const
        {
            g.setFont (Pequod::body (13.0f));
            g.setColour (Pequod::Colours::textDim);
            for (auto& l : labels) g.drawFittedText (l.first, l.second.withTrimmedRight (8), juce::Justification::centredLeft, 2);
            for (auto& h : headings)
            {
                g.setColour (Pequod::Colours::mustard);
                g.setFont (Pequod::heading (13.0f));
                g.drawText (h.first, h.second, juce::Justification::bottomLeft);
                g.setColour (Pequod::Colours::line);
                g.fillRect (h.second.getX(), h.second.getBottom() + 1, h.second.getWidth(), 1);
            }
        }
    };

    void setWidth (juce::Component& c, int w) { c.getProperties().set ("w", w); }
}

//==============================================================================
/** Colour picker: a row of LED colours, plus "the layer's colour" and "the effect's colour". */
class SwatchRow : public juce::Component, public juce::SettableTooltipClient
{
public:
    struct Swatch { juce::String value; juce::Colour colour; juce::String name; juce::String letter; };

    std::function<void (const juce::String&)> onPick;
    juce::String current;
    std::vector<Swatch> swatches;

    juce::Rectangle<float> cell (int i) const
    {
        const float d = juce::jmin ((float) getHeight(), (float) getWidth() / (float) juce::jmax (1, (int) swatches.size()));
        return juce::Rectangle<float> ((float) i * d, 0.0f, d, d).reduced (2.5f);
    }

    void paint (juce::Graphics& g) override
    {
        for (int i = 0; i < (int) swatches.size(); ++i)
        {
            auto& sw = swatches[(size_t) i];
            auto r = cell (i);
            g.setColour (sw.colour);
            g.fillEllipse (r.reduced (2.0f));
            if (sw.letter.isNotEmpty())
            {
                g.setColour (Pequod::Colours::rail);
                g.setFont (Pequod::heading (sw.letter.length() > 1 ? 9.0f : 11.0f));
                g.drawText (sw.letter, r, juce::Justification::centred);
            }
            if (sw.value.equalsIgnoreCase (current))
            {
                g.setColour (Pequod::Colours::text);
                g.drawEllipse (r, 2.0f);
            }
        }
    }

    int indexAt (juce::Point<float> p) const
    {
        for (int i = 0; i < (int) swatches.size(); ++i)
            if (cell (i).contains (p)) return i;
        return -1;
    }

    void mouseMove (const juce::MouseEvent& e) override
    {
        const int i = indexAt (e.position);
        setTooltip (i >= 0 ? swatches[(size_t) i].name : juce::String());
    }

    void mouseDown (const juce::MouseEvent& e) override
    {
        const int i = indexAt (e.position);
        if (i < 0) return;
        current = swatches[(size_t) i].value;
        repaint();
        if (onPick) onPick (current);
    }
};

//==============================================================================
/** The controller as it sits on the floor: one row of boards, all the same height, laid out
    like the hardware (2x2 footswitch, vertical 4-switch strip, pedal, knob). Drag a board by
    its top bar to move it to where it really is. */
class ControllerPanel::BoardView : public juce::Component
{
public:
    explicit BoardView (ControllerManager& m) : mgr (m) {}

    std::function<void (const juce::String&, int)> onSelect;
    juce::String selUid;
    int selCode = -1;

    struct Hit { juce::String uid; int code; juce::Rectangle<float> r; };
    struct Card { const ControllerUnit* unit; juce::Rectangle<float> r; };

    /** Boards in the order they are on the floor (the controller's slot setting). */
    std::vector<const ControllerUnit*> orderedUnits() const
    {
        std::vector<const ControllerUnit*> list;
        for (auto& u : mgr.getInfo().units) list.push_back (&u);
        auto slotOf = [this] (const ControllerUnit* u)
        {
            auto v = mgr.get ("units." + u->uid + ".slot");
            return v.isVoid() ? u->slot : (int) v;
        };
        std::stable_sort (list.begin(), list.end(), [&] (auto* a, auto* b) { return slotOf (a) < slotOf (b); });
        return list;
    }

    void paint (juce::Graphics& g) override
    {
        g.setColour (Pequod::Colours::well);
        g.fillRoundedRectangle (getLocalBounds().toFloat(), 6.0f);

        hits.clear();
        if (mgr.getInfo().units.empty())
        {
            g.setColour (Pequod::Colours::textDim);
            g.setFont (Pequod::body (15.0f));
            g.drawFittedText ("No boards found on the controller.\nCheck the cables between the boards, then press FIND BOARDS AGAIN under SETTINGS.",
                              getLocalBounds().reduced (30), juce::Justification::centred, 4);
            return;
        }

        auto cards = layoutCards();
        for (size_t i = 0; i < cards.size(); ++i)
        {
            if ((int) i == dragIndex) continue;
            drawCard (g, cards[i], false);
        }
        if (dragIndex >= 0 && dragIndex < (int) cards.size())
        {
            // where it will land
            const int to = dropIndex (cards);
            const float x = to < (int) cards.size() ? cards[(size_t) to].r.getX() - 8.0f : cards.back().r.getRight() + 8.0f;
            g.setColour (Pequod::Colours::mustard);
            g.fillRect (juce::Rectangle<float> (x - 1.5f, cards[0].r.getY(), 3.0f, cards[0].r.getHeight()));
            auto c = cards[(size_t) dragIndex];
            c.r = c.r.withX (c.r.getX() + dragOffset);
            drawCard (g, c, true);
        }

        // legend
        auto legend = juce::Rectangle<float> (pad, pad + cardH + 6.0f, juce::jmax ((float) getWidth() - pad * 2.0f, 700.0f), 18.0f);
        auto ring = legend.removeFromLeft (18.0f).withSizeKeepingCentre (14.0f, 14.0f);
        drawGlobalRing (g, ring.expanded (2.0f));
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::body (12.0f));
        g.drawText ("  GLOBAL: the same in every preset.     Press a switch on the controller to pick it.     Drag a board's top bar to move it.",
                    legend, juce::Justification::centredLeft);
    }

    void mouseDown (const juce::MouseEvent& e) override
    {
        pressCard = -1;
        dragIndex = -1;
        dragOffset = 0.0f;
        auto cards = layoutCards();
        for (size_t i = 0; i < cards.size(); ++i)
            if (cards[i].r.contains (e.position)) pressCard = (int) i;
        pressedOnBar = pressCard >= 0 && e.position.y < cards[(size_t) pressCard].r.getY() + 30.0f;
    }

    void mouseDrag (const juce::MouseEvent& e) override
    {
        if (pressCard < 0) return;
        if (dragIndex < 0 && (pressedOnBar || e.getDistanceFromDragStart() > 12) && std::abs (e.getDistanceFromDragStartX()) > 6)
            dragIndex = pressCard;
        if (dragIndex >= 0)
        {
            dragOffset = (float) e.getDistanceFromDragStartX();
            setMouseCursor (juce::MouseCursor::DraggingHandCursor);
            repaint();
        }
    }

    void mouseUp (const juce::MouseEvent& e) override
    {
        setMouseCursor (juce::MouseCursor::NormalCursor);
        if (dragIndex >= 0)
        {
            auto cards = layoutCards();
            const int to = dropIndex (cards);
            auto order = orderedUnits();
            auto* moving = order[(size_t) dragIndex];
            order.erase (order.begin() + dragIndex);
            order.insert (order.begin() + (to > dragIndex ? to - 1 : to), moving);
            for (size_t i = 0; i < order.size(); ++i)
                if ((int) mgr.get ("units." + order[i]->uid + ".slot") != (int) i || mgr.get ("units." + order[i]->uid + ".slot").isVoid())
                    mgr.set ("units." + order[i]->uid + ".slot", (int) i);
            dragIndex = -1;
            repaint();
            return;
        }
        for (auto& h : hits)
            if (h.r.expanded (4.0f).contains (e.position))
            {
                if (onSelect) onSelect (h.uid, h.code);
                return;
            }
    }

private:
    ControllerManager& mgr;
    std::vector<Hit> hits;
    int pressCard = -1, dragIndex = -1;
    bool pressedOnBar = false;
    float dragOffset = 0.0f;

    enum class Shape { grid, column, pedal, pedalSwitch, encoder, other };

    static Shape shapeOf (const ControllerUnit& u)
    {
        if (u.kind == "footswitch") return u.type == 21 ? Shape::column : Shape::grid;
        if (u.kind == "button")       return Shape::grid;
        if (u.kind == "pedal")        return Shape::pedal;
        if (u.kind == "pedal_switch") return Shape::pedalSwitch;
        if (u.kind == "encoder")      return Shape::encoder;
        return Shape::other;
    }

    static juce::String kindTitle (const ControllerUnit& u)
    {
        switch (shapeOf (u))
        {
            case Shape::grid:        return u.kind == "button" ? "BUTTONS" : "FOOTSWITCH";
            case Shape::column:      return "VERTICAL";
            case Shape::pedal:       return "PEDAL";
            case Shape::pedalSwitch: return "PEDAL";
            case Shape::encoder:     return "KNOB";
            case Shape::other:       return "BOARD";
        }
        return {};
    }

public:
    // Sizes and positions measured from a photo of the real controller: a board is 595 units deep,
    // a 2x2 footswitch board 437 wide, the vertical strip 120, the expression pedal 400.
    // Everything is drawn to that one scale, so every switch is the same size.
    static constexpr float photoDepth = 595.0f, faceH = 360.0f, k = faceH / photoDepth;
    static constexpr float switchSize = 64.0f * k, barH = 24.0f, labelH = 20.0f;
    static constexpr float cardH = barH + faceH;
    static constexpr float pad = 14.0f, gap = 10.0f, legendH = 26.0f;

    int getRequiredWidth() const
    {
        float x = pad;
        for (auto* u : orderedUnits()) x += widthFor (*u) + gap;
        return (int) (x - gap + pad);
    }
    static int getRequiredHeight() { return (int) (cardH + pad * 2.0f + legendH); }

private:
    static float widthFor (const ControllerUnit& u)
    {
        switch (shapeOf (u))
        {
            case Shape::grid:        return 437.0f * k;
            case Shape::column:      return 120.0f * k;
            case Shape::pedal:
            case Shape::pedalSwitch: return 400.0f * k;
            case Shape::encoder:     return 220.0f * k;
            case Shape::other:       return 120.0f * k;
        }
        return 120.0f * k;
    }

    std::vector<Card> layoutCards() const
    {
        std::vector<Card> cards;
        float x = pad;
        for (auto* u : orderedUnits())
        {
            const float w = widthFor (*u);
            cards.push_back ({ u, { x, pad, w, cardH } });
            x += w + gap;
        }
        return cards;
    }

    int dropIndex (const std::vector<Card>& cards) const
    {
        if (dragIndex < 0) return 0;
        const float centre = cards[(size_t) dragIndex].r.getCentreX() + dragOffset;
        int to = 0;
        for (size_t i = 0; i < cards.size(); ++i)
            if (centre > cards[i].r.getCentreX()) to = (int) i + 1;
        if (to > dragIndex) to = juce::jmax (to, dragIndex + 1);
        return juce::jlimit (0, (int) cards.size(), to);
    }

    static void drawGlobalRing (juce::Graphics& g, juce::Rectangle<float> r)
    {
        juce::Path ring;
        ring.addEllipse (r);
        juce::Path dashed;
        const float dashes[] = { 3.0f, 3.0f };
        juce::PathStrokeType (1.4f).createDashedStroke (dashed, ring, dashes, 2);
        g.setColour (Pequod::Colours::text.withAlpha (0.85f));
        g.fillPath (dashed);
    }

    void drawCard (juce::Graphics& g, const Card& card, bool lifted)
    {
        auto& u = *card.unit;
        auto r = card.r;
        if (lifted)
        {
            g.setColour (juce::Colours::black.withAlpha (0.4f));
            g.fillRoundedRectangle (r.translated (4.0f, 6.0f), 6.0f);
        }
        g.setGradientFill (juce::ColourGradient (Pequod::Colours::panelHi, r.getX(), r.getY(), Pequod::Colours::panel, r.getX(), r.getBottom(), false));
        g.fillRoundedRectangle (r, 6.0f);
        g.setColour (lifted ? Pequod::Colours::mustard : u.uid == selUid ? Pequod::Colours::mustardLo : Pequod::Colours::line);
        g.drawRoundedRectangle (r, 6.0f, lifted ? 2.0f : 1.0f);

        // top bar: what it is, where it is, grip
        auto bar = r.reduced (8.0f, 6.0f).removeFromTop (18.0f);
        auto grip = bar.removeFromRight (12.0f);
        g.setColour (Pequod::Colours::textDim);
        for (int row = 0; row < 3; ++row)
            for (int col = 0; col < 2; ++col)
                g.fillEllipse (grip.getX() + 2.0f + (float) col * 5.0f, grip.getCentreY() - 6.0f + (float) row * 5.0f, 2.2f, 2.2f);
        g.setColour (Pequod::Colours::mustard);
        g.setFont (Pequod::heading (11.0f));
        g.drawFittedText (kindTitle (u), bar.withTrimmedRight (2.0f).toNearestInt(), juce::Justification::centredLeft, 1, 0.55f);

        auto face = r.withTrimmedTop (barH);
        auto codes = ControllerManager::controlCodes (u);
        // a switch centred at (fx, fy) of the face, with its label underneath
        auto at = [&] (float fx, float fy)
        {
            const float zone = switchSize + 12.0f;
            const float w = juce::jmax (switchSize + 16.0f, juce::jmin (90.0f, face.getWidth()));
            return juce::Rectangle<float> (face.getX() + fx * face.getWidth() - w * 0.5f,
                                           face.getY() + fy * face.getHeight() - zone * 0.5f, w, zone + labelH);
        };
        auto part = [&] (float x0, float y0, float x1, float y1)
        {
            return juce::Rectangle<float>::leftTopRightBottom (face.getX() + x0 * face.getWidth(), face.getY() + y0 * face.getHeight(),
                                                               face.getX() + x1 * face.getWidth(), face.getY() + y1 * face.getHeight());
        };
        switch (shapeOf (u))
        {
            case Shape::grid:          // 2 x 2 as on the boards
            {
                const float xs[2] = { 0.26f, 0.74f }, ys[2] = { 0.29f, 0.80f };
                for (int i = 0; i < 4; ++i)
                    drawControl (g, u, i, at (xs[i % 2], ys[i / 2]));
                break;
            }
            case Shape::column:        // the strip of 4, switch 1 at the top
            {
                const float ys[4] = { 0.126f, 0.387f, 0.647f, 0.899f };
                for (int i = 0; i < 4; ++i)
                    drawControl (g, u, i, at (0.5f, ys[i]));
                break;
            }
            case Shape::pedal:
                drawControl (g, u, Mapping::ctrlPedal, part (0.07f, 0.03f, 0.93f, 0.97f));
                break;
            case Shape::pedalSwitch:
                drawControl (g, u, Mapping::ctrlPedal, part (0.07f, 0.03f, 0.93f, 0.74f));
                drawControl (g, u, 0, at (0.5f, 0.86f));
                break;
            case Shape::encoder:
                drawControl (g, u, Mapping::ctrlEncoderTurn, part (0.05f, 0.12f, 0.95f, 0.55f));
                drawControl (g, u, Mapping::ctrlEncoderPress, at (0.5f, 0.76f));
                break;
            case Shape::other:
                for (int i = 0; i < juce::jmin (4, codes.size()); ++i)
                    drawControl (g, u, codes[i], at (0.5f, 0.126f + 0.26f * (float) i));
                break;
        }
    }

    void drawControl (juce::Graphics& g, const ControllerUnit& u, int code, juce::Rectangle<float> a)
    {
        const bool selected = u.uid == selUid && code == selCode;
        const bool down = mgr.isPressed (u.uid, code);
        const int lay = mgr.getLayer();
        auto* m = mgr.mappingFor (u.uid, code, lay);
        const bool global = m != nullptr && ! m->perPreset;
        auto labelArea = a.removeFromBottom (labelH);
        juce::Rectangle<float> hit;

        if (code == Mapping::ctrlPedal)
        {
            auto bar = a.reduced (2.0f);
            g.setColour (Pequod::Colours::rail);
            g.fillRoundedRectangle (bar, 6.0f);
            const float v = mgr.pedalValue (u.uid);
            g.setColour (Pequod::Colours::mustard.withAlpha (0.85f));
            g.fillRoundedRectangle (bar.reduced (4.0f).withTrimmedTop ((1.0f - v) * (bar.getHeight() - 8.0f)), 4.0f);
            for (int k = 1; k < 10; ++k)
            {
                g.setColour (juce::Colours::black.withAlpha (0.35f));
                g.fillRect (bar.getX() + 5.0f, bar.getY() + bar.getHeight() * (float) k / 10.0f, bar.getWidth() - 10.0f, 1.0f);
            }
            hit = bar;
            if (global) drawGlobalRing (g, bar.expanded (5.0f));
        }
        else if (code == Mapping::ctrlEncoderTurn)
        {
            const float d = juce::jmin (a.getWidth() * 0.8f, a.getHeight() - 4.0f);
            auto k = a.withSizeKeepingCentre (d, d);
            g.setGradientFill (juce::ColourGradient (juce::Colour (0xff4a4a4a), k.getX(), k.getY(), juce::Colour (0xff141414), k.getRight(), k.getBottom(), false));
            g.fillEllipse (k);
            g.setColour (Pequod::Colours::line);
            g.drawEllipse (k, 1.0f);
            g.setColour (Pequod::Colours::mustard);
            g.drawLine (k.getCentreX(), k.getY() + d * 0.12f, k.getCentreX(), k.getCentreY() - d * 0.1f, 3.0f);
            hit = k;
            if (global) drawGlobalRing (g, k.expanded (5.0f));
        }
        else
        {
            const float d = switchSize;
            auto c = a.withSizeKeepingCentre (d, d);
            const bool hasLed = ControllerManager::hasLed (u, code);
            const auto col = mgr.colourOf (u.uid, code);
            const bool lit = mgr.isLit (u.uid, code);
            if (hasLed)
            {
                if (lit)
                {
                    g.setColour (col.withAlpha (0.25f));
                    g.fillEllipse (c.expanded (d * 0.1f));
                }
                g.setColour (lit ? col : col.withAlpha (0.22f));
                g.drawEllipse (c.reduced (2.0f), juce::jmax (2.0f, d * 0.07f));
            }
            auto cap = c.reduced (d * 0.16f);
            if (down) cap = cap.reduced (2.0f);
            g.setGradientFill (juce::ColourGradient (juce::Colour (down ? 0xff9a9a9a : 0xff707070), cap.getX(), cap.getY(),
                                                     juce::Colour (0xff1c1c1c), cap.getRight(), cap.getBottom(), false));
            g.fillEllipse (cap);
            g.setColour (juce::Colours::black.withAlpha (0.6f));
            g.drawEllipse (cap, 1.0f);
            hit = c;
            if (global) drawGlobalRing (g, c.expanded (d * 0.11f));
        }

        if (selected)
        {
            g.setColour (Pequod::Colours::mustard);
            g.drawRoundedRectangle (hit.expanded (juce::jmax (4.0f, hit.getWidth() * 0.13f)), 8.0f, 2.0f);
        }

        auto label = mgr.shortLabel (u.uid, code, lay);
        g.setColour (label == "-" ? Pequod::Colours::textDim : Pequod::Colours::text);
        g.setFont (Pequod::heading (11.0f));
        g.drawFittedText (label, labelArea.expanded (6.0f, 0.0f).toNearestInt(), juce::Justification::centredTop, 1, 0.7f);

        hits.push_back ({ u.uid, code, hit.getUnion (labelArea) });
    }
};

//==============================================================================
/** Pedal position, with the calibrated range and the raw reading. */
class PedalMeter : public juce::Component
{
public:
    float value = 0.0f;
    int raw = -1, calMin = -1, calMax = -1;

    void paint (juce::Graphics& g) override
    {
        auto r = getLocalBounds().toFloat().reduced (0.0f, 4.0f);
        g.setColour (Pequod::Colours::rail);
        g.fillRoundedRectangle (r, 4.0f);
        g.setColour (Pequod::Colours::mustard);
        g.fillRoundedRectangle (r.reduced (3.0f).withWidth ((r.getWidth() - 6.0f) * juce::jlimit (0.0f, 1.0f, value)), 3.0f);
        auto rawX = [&] (int v) { return r.getX() + r.getWidth() * (float) v / 4095.0f; };
        if (calMin >= 0 && calMax > calMin)
        {
            g.setColour (Pequod::Colours::text.withAlpha (0.25f));
            g.fillRect (juce::Rectangle<float> (rawX (calMin), r.getBottom() - 4.0f, rawX (calMax) - rawX (calMin), 3.0f));
        }
        if (raw >= 0)
        {
            g.setColour (Pequod::Colours::text);
            g.fillRect (rawX (raw) - 1.0f, r.getY(), 2.0f, r.getHeight());
        }
    }
};

//==============================================================================
/** One MIDI message a control sends to other gear: type, number, channel. */
class MidiRow : public juce::Component
{
public:
    std::function<void()> onChange, onRemove;

    MidiRow()
    {
        for (auto* c : std::initializer_list<juce::Component*> { &type, &number, &channel, &remove }) addAndMakeVisible (c);
        styleCombo (type);
        styleCombo (channel);
        number.setSliderStyle (juce::Slider::IncDecButtons);
        number.setTextBoxStyle (juce::Slider::TextBoxLeft, false, 40, 24);
        number.setRange (0, 127, 1);
        for (int ch = 1; ch <= 16; ++ch) channel.addItem ("Ch " + juce::String (ch), ch);
        remove.setTooltip ("Stop sending this");
        type.onChange = number.onValueChange = channel.onChange = [this] { if (onChange) onChange(); };
        remove.onClick = [this] { if (onRemove) onRemove(); };
    }

    void set (const juce::StringArray& ids, const juce::StringArray& names, const juce::var& v, bool encoder)
    {
        typeIds = ids;
        type.clear (juce::dontSendNotification);
        type.addItemList (names, 1);
        type.setSelectedItemIndex (juce::jmax (0, ids.indexOf (v["type"].toString())), juce::dontSendNotification);
        number.setValue ((int) v["num"], juce::dontSendNotification);
        number.setVisible (! (encoder && v["type"].toString() == "pc"));       // turning steps through programs
        channel.setSelectedId (juce::jlimit (1, 16, (int) v["ch"]), juce::dontSendNotification);
        resized();
    }

    juce::var get() const
    {
        auto v = juce::var (new juce::DynamicObject());
        v.getDynamicObject()->setProperty ("type", typeIds[juce::jmax (0, type.getSelectedItemIndex())]);
        v.getDynamicObject()->setProperty ("num", (int) number.getValue());
        v.getDynamicObject()->setProperty ("ch", juce::jmax (1, channel.getSelectedId()));
        return v;
    }

    void resized() override
    {
        auto r = getLocalBounds();
        remove.setBounds (r.removeFromRight (32));
        r.removeFromRight (6);
        channel.setBounds (r.removeFromRight (78));
        r.removeFromRight (6);
        if (number.isVisible()) { number.setBounds (r.removeFromRight (108)); r.removeFromRight (6); }
        type.setBounds (r);
    }

private:
    juce::ComboBox type, channel;
    juce::Slider number;
    juce::TextButton remove { "X" };
    juce::StringArray typeIds;
};

/** The layer change a control makes. */
class LayerRow : public juce::Component
{
public:
    std::function<void()> onChange, onRemove;

    LayerRow()
    {
        for (auto* c : std::initializer_list<juce::Component*> { &action, &target, &remove }) addAndMakeVisible (c);
        styleCombo (action);
        styleCombo (target);
        for (int l = 1; l <= 4; ++l) target.addItem ("Layer " + juce::String (l), l);
        action.onChange = target.onChange = [this] { if (onChange) onChange(); };
        remove.onClick = [this] { if (onRemove) onRemove(); };
    }

    void set (const juce::var& v, int numLayers)
    {
        juce::ignoreUnused (numLayers);
        auto t = v["type"].toString();
        action.clear (juce::dontSendNotification);
        if (t == "turn") { action.addItem ("Turning changes layer", 1); action.setSelectedId (1, juce::dontSendNotification); ids = { "turn" }; }
        else
        {
            ids = { "next", "prev", "set", "hold" };
            action.addItemList ({ "Next layer", "Previous layer", "Go to layer", "Hold for layer (back when released)" }, 1);
            action.setSelectedItemIndex (juce::jmax (0, ids.indexOf (t)), juce::dontSendNotification);
        }
        target.setSelectedId (juce::jlimit (1, 4, (int) v["num"]), juce::dontSendNotification);
        target.setVisible (t == "set" || t == "hold");
        resized();
    }

    juce::var get() const
    {
        auto v = juce::var (new juce::DynamicObject());
        v.getDynamicObject()->setProperty ("type", ids[juce::jmax (0, action.getSelectedItemIndex())]);
        v.getDynamicObject()->setProperty ("num", juce::jmax (1, target.getSelectedId()));
        return v;
    }

    void resized() override
    {
        auto r = getLocalBounds();
        remove.setBounds (r.removeFromRight (32));
        r.removeFromRight (6);
        if (target.isVisible()) { target.setBounds (r.removeFromRight (100)); r.removeFromRight (6); }
        action.setBounds (r);
    }

private:
    juce::ComboBox action, target;
    juce::TextButton remove { "X" };
    juce::StringArray ids;
};

//==============================================================================
/** What the selected control does. */
class ControllerPanel::Inspector : public juce::Component
{
public:
    Inspector (ControllerManager& m, HostEngine& e, ControlSurface& c) : mgr (m), engine (e), control (c)
    {
        for (auto* comp : std::initializer_list<juce::Component*> { &title, &sub, &identifyButton, &presetCaption, &modeBox, &functionButton,
                                                                   &latchButton, &momentaryButton, &midiTypeBox, &numberSlider, &channelBox,
                                                                   &layerActionBox, &layerNumBox, &swatches, &ledModeBox, &meter, &everyPresetButton, &thisPresetButton,
                                                                   &calibrateButton, &invertToggle, &hint })
            addChildComponent (comp);

        title.setFont (Pequod::heading (20.0f));
        title.setColour (juce::Label::textColourId, Pequod::Colours::text);
        sub.setFont (Pequod::body (12.0f));
        sub.setColour (juce::Label::textColourId, Pequod::Colours::textDim);
        hint.setFont (Pequod::body (13.0f));
        hint.setColour (juce::Label::textColourId, Pequod::Colours::mustard);
        hint.setJustificationType (juce::Justification::topLeft);

        identifyButton.setTooltip ("Flash it on the controller");
        identifyButton.onClick = [this] { mgr.identify (uid, code); };
        setWidth (identifyButton, 80);

        presetCaption.setFont (Pequod::body (12.0f));
        presetCaption.setColour (juce::Label::textColourId, Pequod::Colours::textDim);

        for (auto* b : { &modeBox, &midiTypeBox, &channelBox, &layerActionBox, &layerNumBox, &ledModeBox }) styleCombo (*b);
        modeBox.onChange = [this] { modeChosen(); };

        functionButton.setColour (juce::TextButton::buttonColourId, Pequod::Colours::well);
        functionButton.onClick = [this] { showFunctionMenu (-1, &functionButton); };
        addFunctionButton.setColour (juce::TextButton::buttonColourId, Pequod::Colours::panel);
        addFunctionButton.setTooltip ("One press can do several things, e.g. switch two effects, or an effect and a plugin knob");
        addFunctionButton.onClick = [this] { showFunctionMenu (-1, &addFunctionButton); };
        addChildComponent (addFunctionButton);
        addChildComponent (layerRow);
        layerRow.onChange = [this] { mgr.setLayerAction (uid, code, mgr.getLayer(), layerRow.get()); };
        layerRow.onRemove = [this]
        {
            juce::Component::SafePointer<Inspector> safePtr (this);
                juce::MessageManager::callAsync ([safe = safePtr]
            {
                if (safe == nullptr) return;
                safe->mgr.setLayerAction (safe->uid, safe->code, safe->mgr.getLayer(), {});
                safe->refresh();
            });
        };
        for (auto* b : { &latchButton, &momentaryButton })
        {
            b->setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
            b->setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
            setWidth (*b, 118);
        }
        latchButton.setTooltip ("Press once for on, press again for off");
        momentaryButton.setTooltip ("On only while your foot is down");
        latchButton.onClick     = [this] { setLatching (true); };
        momentaryButton.onClick = [this] { setLatching (false); };

        midiTypeBox.onChange = [this] { setAction ("mode", midiModes()[midiTypeBox.getSelectedItemIndex()]); refresh(); };
        numberSlider.setSliderStyle (juce::Slider::IncDecButtons);
        numberSlider.setTextBoxStyle (juce::Slider::TextBoxLeft, false, 60, 26);
        numberSlider.setRange (0, 127, 1);
        numberSlider.onValueChange = [this] { setAction ("num", (int) numberSlider.getValue()); };
        setWidth (numberSlider, 150);
        for (int ch = 1; ch <= 16; ++ch) channelBox.addItem ("Channel " + juce::String (ch), ch);
        channelBox.onChange = [this] { setAction ("ch", channelBox.getSelectedId()); };
        setWidth (channelBox, 120);

        layerActionBox.addItemList ({ "Next layer", "Previous layer", "Go to layer", "Hold for layer (back when released)" }, 1);
        layerActionBox.onChange = [this]
        {
            static const char* modes[] = { "layer_next", "layer_prev", "layer_set", "layer_hold" };
            setAction ("mode", modes[juce::jlimit (0, 3, layerActionBox.getSelectedItemIndex())]);
            refresh();
        };
        for (int l = 1; l <= 4; ++l) layerNumBox.addItem ("Layer " + juce::String (l), l);
        layerNumBox.onChange = [this] { setAction ("num", layerNumBox.getSelectedId()); };
        setWidth (layerNumBox, 100);

        swatches.onPick = [this] (const juce::String& col) { pickColour (col); };

        for (auto* b : { &everyPresetButton, &thisPresetButton })
        {
            b->setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
            b->setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
            setWidth (*b, 118);
        }
        everyPresetButton.setTooltip ("GLOBAL: this switch does the same in every preset, whichever preset is loaded");
        thisPresetButton.setTooltip ("Each preset gives this switch its own functions");
        everyPresetButton.onClick = [this] { setScope (false); };
        thisPresetButton.onClick  = [this] { setScope (true); };

        ledModeBox.addItemList ({ "Shows what it controls", "Always on", "Always off", "Follows incoming MIDI" }, 1);
        ledModeBox.onChange = [this]
        {
            static const char* modes[] = { "auto", "on", "off", "midi" };
            mgr.set (ControllerManager::controlPath (uid, code) + ".led", modes[juce::jlimit (0, 3, ledModeBox.getSelectedItemIndex())]);
        };

        calibrateButton.onClick = [this] { toggleCalibration(); };
        setWidth (calibrateButton, 110);
        invertToggle.onClick = [this]
        {
            const bool enc = code == Mapping::ctrlEncoderTurn;
            mgr.set ("units." + uid + (enc ? ".encoder.invert" : ".pedal.invert"), invertToggle.getToggleState());
        };
    }

    ~Inspector() override { stopCalibration (false); }

    void setControl (const juce::String& newUid, int newCode)
    {
        if (newUid == uid && newCode == code) return;
        stopCalibration (false);
        uid = newUid;
        code = newCode;
        refresh();
    }

    juce::String getUid() const { return uid; }
    int getCode() const { return code; }

    /** Re-reads everything from the manager. */
    void refresh()
    {
        const ControllerUnit* u = mgr.getInfo().findUnit (uid);
        for (auto* c : getChildren()) c->setVisible (false);
        form.clear();
        hint.setText ({}, juce::dontSendNotification);

        if (u == nullptr || ! mgr.hasConfig())
        {
            hint.setText (mgr.hasConfig() ? "Pick a switch, pedal or knob on the left - or just press one on the controller."
                                          : "Reading the controller's settings...", juce::dontSendNotification);
            hint.setVisible (true);
            form.wide ({ &hint }, 60);
            resized();
            repaint();
            if (onLayoutChanged) onLayoutChanged();
            return;
        }

        const int lay = mgr.getLayer();
        const bool sw = ControllerManager::isSwitch (code), pedal = code == Mapping::ctrlPedal, turn = code == Mapping::ctrlEncoderTurn;

        title.setText (ControllerManager::controlName (*u, code).toUpperCase(), juce::dontSendNotification);
        sub.setText (boardTitle (*u) + (mgr.getNumLayers() > 1 ? "   -   layer " + juce::String (lay + 1) : juce::String()),
                     juce::dontSendNotification);
        form.wide ({ &title, &identifyButton }, 30);
        form.wide ({ &sub }, 16);

        // What it does: PEQUOD functions, MIDI to other gear and a layer change, in any combination
        form.heading ("WHAT IT DOES");
        auto all = mgr.mappingsFor (uid, code, lay);
        auto* m = all.empty() ? nullptr : all.front();
        auto midi = mgr.midiOf (uid, code, lay);
        auto layerAct = mgr.layerOf (uid, code, lay);
        const int total = (int) all.size() + midi.size() + (layerAct.isVoid() ? 0 : 1);

        while (funcButtons.size() < (int) all.size())
        {
            const int i = funcButtons.size();
            auto* f = funcButtons.add (new juce::TextButton());
            f->setColour (juce::TextButton::buttonColourId, Pequod::Colours::well);
            f->onClick = [this, i] { showFunctionMenu (i, funcButtons[i]); };
            addChildComponent (f);
            auto* x = removeButtons.add (new juce::TextButton ("X"));
            x->setTooltip ("Remove this function");
            x->onClick = [this, i] { removeFunctionAt (i); };
            setWidth (*x, 32);
            addChildComponent (x);
        }
        for (int i = 0; i < (int) all.size(); ++i)
        {
            funcButtons[i]->setButtonText (describe (*all[(size_t) i]));
            form.row (i == 0 ? juce::String ("PEQUOD") : juce::String(), { funcButtons[i], removeButtons[i] }, 32);
        }

        while (midiRows.size() < midi.size())
        {
            const int i = midiRows.size();
            auto* row = midiRows.add (new MidiRow());
            row->onChange = [this, i]
            {
                auto list = mgr.midiOf (uid, code, mgr.getLayer());
                if (! juce::isPositiveAndBelow (i, list.size())) return;
                list.set (i, midiRows[i]->get());
                mgr.setMidi (uid, code, mgr.getLayer(), list);
            };
            row->onRemove = [this, i]
            {
                juce::Component::SafePointer<Inspector> safePtr (this);
                juce::MessageManager::callAsync ([safe = safePtr, i]
                {
                    if (safe == nullptr) return;
                    auto list = safe->mgr.midiOf (safe->uid, safe->code, safe->mgr.getLayer());
                    if (juce::isPositiveAndBelow (i, list.size())) list.remove (i);
                    safe->mgr.setMidi (safe->uid, safe->code, safe->mgr.getLayer(), list);
                    safe->refresh();
                });
            };
            addChildComponent (row);
        }
        for (int i = 0; i < midi.size(); ++i)
        {
            midiRows[i]->set (midiModes(), midiModeNames(), midi[i], code == Mapping::ctrlEncoderTurn);
            form.row (i == 0 ? juce::String ("MIDI out") : juce::String(), { midiRows[i] }, 30);
        }

        if (! layerAct.isVoid())
        {
            layerRow.set (layerAct, mgr.getNumLayers());
            form.row ("Layer", { &layerRow }, 30);
        }

        addFunctionButton.setButtonText (total == 0 ? "Choose what it does..." : "+ ADD ANOTHER FUNCTION");
        form.row (total == 0 ? juce::String ("Function") : juce::String(), { &addFunctionButton }, total == 0 ? 32 : 26);

        if (total > 0 || control.isControllerPerPreset (uid, code, lay))
        {
            const bool perPreset = control.isControllerPerPreset (uid, code, lay);
            everyPresetButton.setToggleState (! perPreset, juce::dontSendNotification);
            thisPresetButton.setToggleState (perPreset, juce::dontSendNotification);
            form.row ("Applies to", { &everyPresetButton, &thisPresetButton });
            presetCaption.setText (perPreset ? "Each preset has its own functions, MIDI and layer change - editing " + presetName()
                                             : juce::String ("The same in every preset"),
                                   juce::dontSendNotification);
            form.row ({}, { &presetCaption }, 16);
        }

        const bool pequodLatches = std::any_of (all.begin(), all.end(), [] (const Mapping* f)
        {
            return ! (f->target == Mapping::Target::presetNext || f->target == Mapping::Target::presetPrev
                      || f->target == Mapping::Target::presetSelect || f->target == Mapping::Target::bankNext
                      || f->target == Mapping::Target::bankPrev);
        });
        const bool midiLatches = std::any_of (midi.begin(), midi.end(), [] (const juce::var& v)
        {
            return v["type"].toString() == "cc" || v["type"].toString() == "note";
        });
        if (sw && (total == 0 || pequodLatches || midiLatches))
            showLatch (mgr.isLatching (uid, code, lay));

        juce::StringArray hints;
        if (m != nullptr && ! m->perPreset && m->isRackMapping() && m->slotUid == 0)
            hints.add (m->pluginName + " isn't in this preset, so its function does nothing here.");
        else if (m != nullptr && ! m->perPreset && (m->target == Mapping::Target::bypass || m->target == Mapping::Target::param))
            hints.add ("Follows " + m->pluginName + (m->pluginOccurrence > 0 ? " #" + juce::String (m->pluginOccurrence + 1) : juce::String())
                       + " in every preset that has it.");
        if (! midi.isEmpty()) hints.add (midiOutHint());
        if (! layerAct.isVoid() && mgr.getNumLayers() < 2) hints.add ("There is only one layer. Add more under SETTINGS > Layers.");
        const int kinds = (all.empty() ? 0 : 1) + (midi.isEmpty() ? 0 : 1) + (layerAct.isVoid() ? 0 : 1);
        if ((kinds > 1 || midi.size() > 1) && ! mgr.supportsCombos())
            hints.add ("The controller needs firmware 2.1 to do several of these at once - update it under SETTINGS.");
        if (! hints.isEmpty()) showHint (hints.joinIntoString ("\n"));

        // LED
        if (ControllerManager::hasLed (*u, code))
        {
            form.column();
            form.heading ("LIGHT");
            buildSwatches (lay);
            form.row ("Colour", { &swatches }, 30);
            auto led = mgr.get (ControllerManager::controlPath (uid, code) + ".led").toString();
            static const juce::StringArray ledModes { "auto", "on", "off", "midi" };
            ledModeBox.setSelectedItemIndex (juce::jmax (0, ledModes.indexOf (led)), juce::dontSendNotification);
            form.row ("Light", { &ledModeBox });
        }

        // Pedal / encoder hardware
        if (pedal)
        {
            form.column();
            form.heading ("PEDAL");
            form.row ("Position", { &meter, &calibrateButton }, 30);
            calibrateButton.setButtonText (calibrating ? "DONE" : "CALIBRATE");
            invertToggle.setButtonText ("Reverse direction");
            invertToggle.setToggleState ((bool) mgr.get ("units." + uid + ".pedal.invert"), juce::dontSendNotification);
            form.row ({}, { &invertToggle });
            if (calibrating) showHint ("Rock the pedal all the way back and all the way forward, then press DONE.");
        }
        if (turn)
        {
            form.column();
            form.heading ("KNOB");
            invertToggle.setButtonText ("Reverse direction");
            invertToggle.setToggleState ((bool) mgr.get ("units." + uid + ".encoder.invert"), juce::dontSendNotification);
            form.row ({}, { &invertToggle });
        }

        if (control.getLearnState() != ControlSurface::LearnState::off && hint.getText().isEmpty())
            showHint (control.getLearnStatusText().fromFirstOccurrenceOf ("LEARN: ", false, false));

        for (auto& r : form.rows) for (auto* c : r.comps) c->setVisible (true);
        resized();
        repaint();
        if (onLayoutChanged) onLayoutChanged();
    }

    std::function<void()> onLayoutChanged;

    void tick()
    {
        if (code == Mapping::ctrlPedal && uid.isNotEmpty())
        {
            meter.value = mgr.pedalValue (uid);
            meter.raw = calibrating ? mgr.pedalRaw (uid) : -1;
            if (calibrating && meter.raw >= 0)
            {
                calMin = calMin < 0 ? meter.raw : juce::jmin (calMin, meter.raw);
                calMax = juce::jmax (calMax, meter.raw);
            }
            meter.calMin = calibrating ? calMin : (int) mgr.get ("units." + uid + ".pedal.min");
            meter.calMax = calibrating ? calMax : (int) mgr.get ("units." + uid + ".pedal.max");
            meter.repaint();
        }
    }

    void paint (juce::Graphics& g) override
    {
        g.setColour (Pequod::Colours::panel);
        g.fillRoundedRectangle (getLocalBounds().toFloat(), 6.0f);
        form.paint (g);
    }

    void resized() override
    {
        form.labelWidth = 92;
        form.layout (getLocalBounds().reduced (16, 14));
    }

    /** Height needed to show everything at this width (the window scrolls if it's smaller). */
    int getIdealHeight (int width)
    {
        form.labelWidth = 92;
        const int h = form.layout (juce::Rectangle<int> (16, 14, juce::jmax (200, width - 32), 100)) + 28;
        resized();
        return h;
    }

private:
    ControllerManager& mgr;
    HostEngine& engine;
    ControlSurface& control;
    juce::String uid;
    int code = -1;
    Form form;

    juce::Label title, sub, hint;
    juce::TextButton identifyButton { "FIND IT" }, functionButton, calibrateButton { "CALIBRATE" }, addFunctionButton { "+ ADD ANOTHER FUNCTION" };
    juce::OwnedArray<juce::TextButton> funcButtons, removeButtons;
    juce::OwnedArray<MidiRow> midiRows;
    LayerRow layerRow;
    juce::Label presetCaption;
    juce::ComboBox modeBox, midiTypeBox, channelBox, layerActionBox, layerNumBox, ledModeBox;
    juce::ToggleButton invertToggle;
    juce::TextButton latchButton { "LATCHING" }, momentaryButton { "MOMENTARY" };
    juce::Slider numberSlider;
    SwatchRow swatches;
    juce::TextButton everyPresetButton { "GLOBAL" }, thisPresetButton { "THIS PRESET" };
    PedalMeter meter;

    bool calibrating = false;
    int calMin = -1, calMax = -1;

    void buildSwatches (int lay)
    {
        swatches.swatches.clear();
        for (auto c : swatchColours)
        {
            auto col = juce::Colour (0xff000000 | c);
            swatches.swatches.push_back ({ hex (col), col, {}, {} });
        }
        swatches.swatches.push_back ({ "layer", mgr.layerColour (lay), "The colour of the layer you're on", "L" });

        if (true)
        {
            auto* m = mgr.mappingFor (uid, code, lay);
            auto fx = m != nullptr ? mgr.effectColour (*m) : juce::Colour();
            swatches.swatches.push_back ({ "fx", fx.isTransparent() ? juce::Colour (0xff9a9a9a) : PluginColours::forLed (fx),
                                           "Automatic: the colour of the effect this switch controls (amps oxblood, drive tangerine, "
                                           "delays ocean...). Switches that don't control an effect use the colour picked here before.", "FX" });
        }

        swatches.current = mgr.usesEffectColour (uid, code, lay) ? juce::String ("fx")
                                                                : mgr.get (ControllerManager::controlPath (uid, code) + ".color").toString();
        swatches.repaint();
    }

    void pickColour (const juce::String& col)
    {
        const int lay = mgr.getLayer();
        if (col == "fx")
            mgr.setUseEffectColour (uid, code, lay, true);
        else
        {
            mgr.setUseEffectColour (uid, code, lay, false);
            mgr.set (ControllerManager::controlPath (uid, code) + ".color", col);
        }
        refresh();
    }

    void setScope (bool perPreset)
    {
        const int lay = mgr.getLayer();
        if (control.isControllerPerPreset (uid, code, lay) == perPreset) { refresh(); return; }
        mgr.setScope (uid, code, lay, perPreset);
        if (mgr.onStatus)
            mgr.onStatus (perPreset ? "This switch now has its own functions in each preset - the current ones stay with " + presetName()
                                    : juce::String ("This switch now does the same in every preset"));
        refresh();
    }

    void showHint (const juce::String& s)
    {
        hint.setText (s, juce::dontSendNotification);
        form.wide ({ &hint }, 36 * (juce::StringArray::fromLines (s).size()) + 10);
    }

    juce::String midiOutHint() const
    {
        const bool usb = mgr.get ("midi.usb").isVoid() || (bool) mgr.get ("midi.usb");
        const bool din = mgr.get ("midi.din").isVoid() || (bool) mgr.get ("midi.din");
        if (usb && din) return "Sent from the controller's USB port and its 5-pin MIDI socket.";
        if (din)        return "Sent from the controller's 5-pin MIDI socket.";
        if (usb)        return "Sent from the controller's USB port.";
        return "MIDI out is switched off under SETTINGS > MIDI out.";
    }

    juce::StringArray midiModes() const
    {
        if (code == Mapping::ctrlPedal)       return { "cc" };
        if (code == Mapping::ctrlEncoderTurn) return { "cc_rel", "cc_abs", "pc" };
        return { "cc", "note", "pc" };
    }

    juce::StringArray midiModeNames() const
    {
        if (code == Mapping::ctrlPedal)       return { "Control change (CC)" };
        if (code == Mapping::ctrlEncoderTurn) return { "CC up/down steps (relative)", "CC value 0-127", "Program change (turn to step)" };
        return { "Control change (CC)", "Note", "Program change" };
    }

    void setAction (const juce::String& field, const juce::var& v)
    {
        mgr.set (ControllerManager::actionPath (uid, code, mgr.getLayer()) + "." + field, v);
    }

    juce::String presetName() const
    {
        auto bank = control.getBankName ? control.getBankName() : juce::String();
        auto preset = control.getPresetName ? control.getPresetName() : juce::String();
        return (bank.isNotEmpty() ? bank + " / " : juce::String()) + preset;
    }

    juce::String boardTitle (const ControllerUnit& u) const
    {
        auto slot = mgr.get ("units." + u.uid + ".slot");
        const int pos = (slot.isVoid() ? u.slot : (int) slot) + 1;
        juce::String k = u.kind == "footswitch" ? (u.type == 21 ? "Vertical footswitch" : "Footswitch")
                       : u.kind == "button" ? "Buttons" : u.kind == "encoder" ? "Knob"
                       : u.kind.startsWith ("pedal") ? "Pedal" : "Board";
        return k + " #" + juce::String (pos);
    }

    void modeChosen()
    {
        const int id = modeBox.getSelectedId();
        const bool turn = code == Mapping::ctrlEncoderTurn;
        juce::String mode = id == 1 ? "pequod" : id == 4 ? "none"
                          : id == 3 ? (turn ? "layer" : "layer_next")
                          : midiModes()[0];
        setAction ("mode", mode);
        if (id == 1 && mgr.mappingFor (uid, code, mgr.getLayer()) == nullptr)
        {
            juce::Component::SafePointer<Inspector> safePtr (this);
            juce::MessageManager::callAsync ([safe = safePtr] { if (safe) safe->showFunctionMenu (-1, &safe->functionButton); });
        }
        refresh();
    }

    void showLatch (bool latching)
    {
        latchButton.setToggleState (latching, juce::dontSendNotification);
        momentaryButton.setToggleState (! latching, juce::dontSendNotification);
        form.row ("Switch", { &latchButton, &momentaryButton });
    }

    void setLatching (bool latching)
    {
        mgr.setLatching (uid, code, mgr.getLayer(), latching);
        refresh();
    }

    juce::String describe (const Mapping& m) const
    {
        switch (m.target)
        {
            case Mapping::Target::bypass:
                if (auto s = engine.findSlot (m.slotUid)) return s->description.name + " on/off";
                return "Effect (not in this preset)";
            case Mapping::Target::stompSlot:
            {
                auto slots = engine.getFlatSlots();
                return "Effect " + juce::String (m.paramIndex) + " on/off"
                     + (m.paramIndex > 0 && m.paramIndex <= (int) slots.size() ? "  (" + slots[(size_t) m.paramIndex - 1]->description.name + ")" : juce::String());
            }
            case Mapping::Target::presetNext:   return code == Mapping::ctrlEncoderTurn ? "Step through presets" : "Next preset";
            case Mapping::Target::presetPrev:   return "Previous preset";
            case Mapping::Target::presetSelect:
            {
                auto names = mgr.getPresetNames ? mgr.getPresetNames() : juce::StringArray();
                return "Preset " + juce::String (m.paramIndex)
                     + (m.paramIndex <= names.size() ? "  (" + names[m.paramIndex - 1] + ")" : juce::String());
            }
            case Mapping::Target::bankNext:     return code == Mapping::ctrlEncoderTurn ? "Step through banks" : "Next bank";
            case Mapping::Target::bankPrev:     return "Previous bank";
            case Mapping::Target::mute:         return "Mute";
            case Mapping::Target::tuner:        return "Tuner";
            case Mapping::Target::inputGain:    return "Input level (IN)";
            case Mapping::Target::outputGain:   return "Master volume (OUT)";
            case Mapping::Target::tapTempo:     return "Tap tempo";
            case Mapping::Target::click:        return "Metronome click on/off";
            case Mapping::Target::tempo:        return "Tempo";
            default:                            return control.describeTarget (m);
        }
    }

    void removeFunctionAt (int i)
    {
        juce::Component::SafePointer<Inspector> safePtr (this);
                juce::MessageManager::callAsync ([safe = safePtr, i]
        {
            if (safe == nullptr) return;
            auto all = safe->mgr.mappingsFor (safe->uid, safe->code, safe->mgr.getLayer());
            if (juce::isPositiveAndBelow (i, (int) all.size()))
            {
                Mapping copy = *all[(size_t) i];
                safe->mgr.removeFunction (copy);
            }
            safe->refresh();
        });
    }

    /** index: which of the control's functions to change, -1 = add one */
    void showFunctionMenu (int index, juce::Component* anchor)
    {
        const int lay = mgr.getLayer();
        std::shared_ptr<Mapping> rep;
        {
            auto all = mgr.mappingsFor (uid, code, lay);
            if (juce::isPositiveAndBelow (index, (int) all.size())) rep = std::make_shared<Mapping> (*all[(size_t) index]);
        }
        const bool sw = ControllerManager::isSwitch (code), turn = code == Mapping::ctrlEncoderTurn;
        auto slots = engine.getFlatSlots();
        auto splits = engine.getSplits();
        auto safe = juce::Component::SafePointer<Inspector> (this);
        const auto u = uid;
        const int c = code;
        auto& m = mgr;
        auto done = [safe] { if (safe) safe->refresh(); };

        juce::PopupMenu menu;
        if (sw)
        {
            juce::PopupMenu fx, presets, splitMenu;
            for (size_t i = 0; i < slots.size(); ++i)
            {
                const int slotUid = slots[i]->uid;
                fx.addItem (juce::String ((int) i + 1) + ".  " + slots[i]->description.name,
                            [&m, rep, u, c, lay, slotUid, done] { m.assign (u, c, lay, Mapping::Target::bypass, slotUid, rep.get()); done(); });
            }
            if (slots.empty()) fx.addItem ("(this preset has no effects)", false, false, nullptr);
            auto names = m.getPresetNames ? m.getPresetNames() : juce::StringArray();
            for (int n = 1; n <= juce::jmax (8, names.size()); ++n)
                presets.addItem (juce::String (n) + ".  " + (n <= names.size() ? names[n - 1] : juce::String ("(empty)")),
                                 [&m, rep, u, c, lay, n, done] { m.assign (u, c, lay, Mapping::Target::presetSelect, n, rep.get()); done(); });
            for (auto* s : splits)
            {
                const int itemUid = s->uid;
                splitMenu.addItem ("Split " + juce::String (engine.getSplitNumber (itemUid)),
                                   [&m, rep, u, c, lay, itemUid, done] { m.assign (u, c, lay, Mapping::Target::splitMix, itemUid, rep.get()); done(); });
            }

            menu.addSectionHeader ("PEQUOD - EFFECTS");
            menu.addSubMenu ("An effect...  on/off", fx);
            menu.addItem ("A plugin knob or switch...  (touch it on the plugin next)", [&m, rep, u, c, lay, done] { m.learnPluginControl (u, c, lay, rep.get()); done(); });
            if (! splits.empty()) menu.addSubMenu ("Split: switch between A and B", splitMenu);
            menu.addSectionHeader ("PRESETS");
            menu.addItem ("Next preset",     [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::presetNext, 0, rep.get()); done(); });
            menu.addItem ("Previous preset", [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::presetPrev, 0, rep.get()); done(); });
            menu.addSubMenu ("Preset...", presets);
            menu.addItem ("Next bank",       [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::bankNext, 0, rep.get()); done(); });
            menu.addItem ("Previous bank",   [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::bankPrev, 0, rep.get()); done(); });
            menu.addSectionHeader ("OUTPUT");
            menu.addItem ("Tuner  (mutes while tuning)", [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::tuner, 0, rep.get()); done(); });
            menu.addItem ("Mute",                        [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::mute, 0, rep.get()); done(); });
            menu.addSectionHeader ("TEMPO");
            menu.addItem ("Tap tempo  (its light flashes in time)", [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::tapTempo, 0, rep.get()); done(); });
            menu.addItem ("Metronome click on/off",                 [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::click, 0, rep.get()); done(); });
        }
        else
        {
            juce::PopupMenu splitMenu;
            for (auto* s : splits)
            {
                const int itemUid = s->uid;
                splitMenu.addItem ("Split " + juce::String (engine.getSplitNumber (itemUid)) + " mix A-B",
                                   [&m, rep, u, c, lay, itemUid, done] { m.assign (u, c, lay, Mapping::Target::splitMix, itemUid, rep.get()); done(); });
            }
            menu.addItem ("A plugin knob...  (touch it on the plugin next)", [&m, rep, u, c, lay, done] { m.learnPluginControl (u, c, lay, rep.get()); done(); });
            if (turn)
            {
                menu.addItem ("Step through presets", [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::presetNext, 0, rep.get()); done(); });
                menu.addItem ("Step through banks",   [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::bankNext, 0, rep.get()); done(); });
            }
            menu.addItem ("Master volume (OUT)", [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::outputGain, 0, rep.get()); done(); });
            menu.addItem ("Input level (IN)",    [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::inputGain, 0, rep.get()); done(); });
            menu.addItem (turn ? "Tempo  (each click = 1 BPM)" : "Tempo  (40 - 240 BPM)",
                          [&m, rep, u, c, lay, done] { m.assign (u, c, lay, Mapping::Target::tempo, 0, rep.get()); done(); });
            if (! splits.empty()) menu.addSubMenu ("Split mix", splitMenu);
        }
        if (rep == nullptr)       // adding: MIDI to other gear and layer changes too, alongside PEQUOD functions
        {
            auto midiNow = m.midiOf (u, c, lay);
            if (midiNow.size() < 4)
            {
                menu.addSectionHeader ("MIDI TO OTHER GEAR");
                auto ids = midiModes();
                auto names = midiModeNames();
                const int legacyNum = (int) m.get (ControllerManager::actionPath (u, c, lay) + ".num");
                const int num = juce::jlimit (0, 127, legacyNum > 0 ? legacyNum : 20);
                for (int i = 0; i < ids.size(); ++i)
                {
                    const auto id = ids[i];
                    menu.addItem (names[i], [&m, u, c, lay, id, num, done]
                    {
                        auto list = m.midiOf (u, c, lay);
                        auto v = juce::var (new juce::DynamicObject());
                        v.getDynamicObject()->setProperty ("type", id);
                        v.getDynamicObject()->setProperty ("num", num);
                        v.getDynamicObject()->setProperty ("ch", 1);
                        list.add (v);
                        m.setMidi (u, c, lay, list);
                        done();
                    });
                }
            }
            if (m.layerOf (u, c, lay).isVoid() && (sw || turn))
            {
                menu.addSectionHeader ("LAYERS");
                auto addLayer = [&m, u, c, lay, done] (const char* type, int num)
                {
                    return [&m, u, c, lay, done, type, num]
                    {
                        auto v = juce::var (new juce::DynamicObject());
                        v.getDynamicObject()->setProperty ("type", type);
                        v.getDynamicObject()->setProperty ("num", num);
                        m.setLayerAction (u, c, lay, v);
                        done();
                    };
                };
                if (turn) menu.addItem ("Turning changes layer", addLayer ("turn", 1));
                else
                {
                    menu.addItem ("Next layer",     addLayer ("next", 1));
                    menu.addItem ("Previous layer", addLayer ("prev", 1));
                    menu.addItem ("Go to layer...", addLayer ("set", 2));
                    menu.addItem ("Hold for layer...  (back when released)", addLayer ("hold", 2));
                }
            }
        }
        if (rep != nullptr)
        {
            menu.addSeparator();
            menu.addItem ("Remove this function", [&m, rep, done] { m.removeFunction (*rep); done(); });
        }
        menu.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (anchor).withMinimumWidth (anchor->getWidth()));
    }

    void toggleCalibration()
    {
        if (calibrating) { stopCalibration (true); refresh(); return; }
        calibrating = true;
        calMin = calMax = -1;
        mgr.watchPedal (uid, true);
        refresh();
    }

    void stopCalibration (bool apply)
    {
        if (! calibrating) return;
        calibrating = false;
        mgr.watchPedal (uid, false);
        if (! apply) return;
        if (calMin >= 0 && calMax - calMin > 150)
        {
            const int margin = (calMax - calMin) / 50;    // so the ends are always reached
            mgr.set ("units." + uid + ".pedal.min", calMin + margin);
            mgr.set ("units." + uid + ".pedal.max", calMax - margin);
            if (mgr.onStatus) mgr.onStatus ("Pedal calibrated");
        }
        else if (mgr.onStatus)
            mgr.onStatus ("The pedal didn't move far enough - calibration not changed");
    }
};

//==============================================================================
/** Everything else stored on the controller. */
class ControllerPanel::SettingsPage : public juce::Component
{
public:
    explicit SettingsPage (ControllerManager& m) : mgr (m)
    {
        for (auto* e : { &nameEditor, &passEditor, &ssid2Editor, &pass2Editor, &hostEditor, &ipEditor }) { styleEditor (*e); addAndMakeVisible (e); }
        for (auto* s : { &brightness, &offGlow, &contrast }) { styleSlider (*s); addAndMakeVisible (s); }
        for (auto* b : { &layersBox, &ssidBox }) { styleCombo (*b); addAndMakeVisible (b); }
        for (auto* t : { &tunerToggle, &usbToggle, &dinToggle, &wifiToggle }) addAndMakeVisible (t);
        for (auto* b : { &renameButton, &scanButton, &rescanButton, &rebootButton, &updateButton, &factoryButton }) addAndMakeVisible (b);
        for (auto* l : { &wifiStatus, &info, &updateLabel })
        {
            l->setFont (Pequod::body (13.0f));
            l->setColour (juce::Label::textColourId, Pequod::Colours::textDim);
            addAndMakeVisible (l);
        }
        for (int i = 0; i < 4; ++i)
        {
            auto* b = layerColourButtons.add (new juce::TextButton ("L" + juce::String (i + 1)));
            b->onClick = [this, i] { pickLayerColour (i); };
            setWidth (*b, 44);
            addAndMakeVisible (b);
        }

        nameEditor.setInputRestrictions (24);
        nameEditor.onReturnKey = [this] { commitName(); };
        renameButton.onClick = [this] { commitName(); };
        setWidth (renameButton, 90);

        brightness.setRange (5, 100, 1);
        brightness.setTextValueSuffix (" %");
        brightness.onValueChange = [this] { mgr.set ("brightness", juce::roundToInt (brightness.getValue() * 2.55)); };
        offGlow.setRange (0, 60, 1);
        offGlow.setTextValueSuffix (" %");
        offGlow.onValueChange = [this] { mgr.set ("offGlow", (int) offGlow.getValue()); };
        contrast.setRange (1, 255, 1);
        contrast.onValueChange = [this] { mgr.set ("display.contrast", (int) contrast.getValue()); };
        tunerToggle.setButtonText ("Show the tuner on the screen whenever PEQUOD is muted");
        tunerToggle.onClick = [this] { mgr.set ("display.tunerWhenMuted", tunerToggle.getToggleState()); };

        layersBox.addItemList ({ "1 layer", "2 layers", "3 layers", "4 layers" }, 1);
        layersBox.onChange = [this] { mgr.set ("layers", layersBox.getSelectedId()); };
        setWidth (layersBox, 120);

        usbToggle.setButtonText ("USB  (the computer and anything plugged in with USB)");
        usbToggle.onClick = [this] { mgr.set ("midi.usb", usbToggle.getToggleState()); };
        dinToggle.setButtonText ("5-pin MIDI socket");
        dinToggle.onClick = [this] { mgr.set ("midi.din", dinToggle.getToggleState()); };

        wifiToggle.setButtonText ("Use Wi-Fi  (PEQUOD can reach the controller without a cable)");
        wifiToggle.onClick = [this] { mgr.set ("wifi.enabled", wifiToggle.getToggleState()); };
        ssidBox.setEditableText (true);
        ssidBox.setTextWhenNothingSelected ("Network name");
        ssidBox.onChange = [this]
        {
            auto ssid = ssidBox.getText().upToLastOccurrenceOf ("  (open)", false, false).trim();
            if (ssid != mgr.get ("wifi.ssid").toString()) mgr.set ("wifi.ssid", ssid);
        };
        scanButton.onClick = [this] { mgr.scanWifi(); };
        setWidth (scanButton, 90);
        passEditor.setPasswordCharacter ((juce::juce_wchar) 0x2022);
        pass2Editor.setPasswordCharacter ((juce::juce_wchar) 0x2022);
        passEditor.onReturnKey = passEditor.onFocusLost = [this] { commitPassword (passEditor, "wifi.pass"); };
        pass2Editor.onReturnKey = pass2Editor.onFocusLost = [this] { commitPassword (pass2Editor, "wifi.pass2"); };
        ssid2Editor.onReturnKey = ssid2Editor.onFocusLost = [this] { if (ssid2Editor.getText().trim() != mgr.get ("wifi.ssid2").toString()) mgr.set ("wifi.ssid2", ssid2Editor.getText().trim()); };
        hostEditor.onReturnKey = hostEditor.onFocusLost = [this]
        {
            auto h = hostEditor.getText().trim().toLowerCase().retainCharacters ("abcdefghijklmnopqrstuvwxyz0123456789-");
            if (h.isNotEmpty() && h != mgr.get ("wifi.hostname").toString()) mgr.set ("wifi.hostname", h);
        };
        ipEditor.setTextToShowWhenEmpty ("found by itself", Pequod::Colours::textDim);
        ipEditor.onReturnKey = ipEditor.onFocusLost = [this] { mgr.setManualAddress (ipEditor.getText()); };

        rescanButton.onClick = [this] { mgr.rescan(); };
        rebootButton.onClick = [this] { mgr.reboot(); };
        updateButton.onClick = [this] { chooseFirmware(); };
        factoryButton.onClick = [this] { confirmFactoryReset(); };
        factoryButton.setColour (juce::TextButton::buttonColourId, Pequod::Colours::danger.withAlpha (0.5f));
        for (auto* b : { &rescanButton, &rebootButton, &updateButton, &factoryButton }) setWidth (*b, 150);

        buildForm();
    }

    void buildForm()
    {
        form.clear();
        form.heading ("GENERAL");
        form.row ("Name", { &nameEditor, &renameButton });
        form.row ("LED brightness", { &brightness });
        form.row ("LEDs when off", { &offGlow });
        form.row ("Screen contrast", { &contrast });
        form.row ({}, { &tunerToggle });
        form.heading ("LAYERS");
        form.row ("Layers", { &layersBox, layerColourButtons[0], layerColourButtons[1], layerColourButtons[2], layerColourButtons[3] });
        form.heading ("MIDI OUT  (switches set to \"Send MIDI to other gear\")");
        form.row ({}, { &usbToggle });
        form.row ({}, { &dinToggle });
        form.heading ("WI-FI");
        form.row ({}, { &wifiToggle });
        form.row ("Network", { &ssidBox, &scanButton });
        form.row ("Password", { &passEditor });
        form.row ("Backup network", { &ssid2Editor });
        form.row ("Backup password", { &pass2Editor });
        form.row ("Device name", { &hostEditor });
        form.row ("Status", { &wifiStatus });
        form.row ("Controller IP", { &ipEditor });
        form.heading ("CONTROLLER");
        form.row ("About", { &info }, 40);
        form.row ({}, { &updateLabel, &updateButton });
        form.row ({}, { &rescanButton });
        form.row ({}, { &rebootButton });
        form.row ({}, { &factoryButton });
        form.labelWidth = 150;
    }

    int getIdealHeight() { return form.layout (juce::Rectangle<int> (24, 8, juce::jmax (300, getWidth() - 48), 100)) + 40; }

    void refresh()
    {
        auto setIfIdle = [] (juce::TextEditor& e, const juce::String& s) { if (! e.hasKeyboardFocus (true)) e.setText (s, false); };
        setIfIdle (nameEditor, mgr.get ("name").toString());
        if (! brightness.isMouseButtonDown()) brightness.setValue (juce::roundToInt ((double) mgr.get ("brightness") / 2.55), juce::dontSendNotification);
        if (! offGlow.isMouseButtonDown())    offGlow.setValue ((int) mgr.get ("offGlow"), juce::dontSendNotification);
        if (! contrast.isMouseButtonDown())   contrast.setValue (mgr.get ("display.contrast").isVoid() ? 200 : (int) mgr.get ("display.contrast"), juce::dontSendNotification);
        tunerToggle.setToggleState (mgr.get ("display.tunerWhenMuted").isVoid() || (bool) mgr.get ("display.tunerWhenMuted"), juce::dontSendNotification);
        layersBox.setSelectedId (mgr.getNumLayers(), juce::dontSendNotification);
        for (int i = 0; i < 4; ++i)
        {
            layerColourButtons[i]->setColour (juce::TextButton::buttonColourId, mgr.layerColour (i).withAlpha (0.8f));
            layerColourButtons[i]->setColour (juce::TextButton::textColourOffId, juce::Colours::black);
            layerColourButtons[i]->setEnabled (i < mgr.getNumLayers());
        }
        usbToggle.setToggleState (mgr.get ("midi.usb").isVoid() || (bool) mgr.get ("midi.usb"), juce::dontSendNotification);
        dinToggle.setToggleState (mgr.get ("midi.din").isVoid() || (bool) mgr.get ("midi.din"), juce::dontSendNotification);

        const bool wifiOn = mgr.get ("wifi.enabled").isVoid() || (bool) mgr.get ("wifi.enabled");
        wifiToggle.setToggleState (wifiOn, juce::dontSendNotification);
        if (! ssidBox.hasKeyboardFocus (true))
        {
            ssidBox.clear (juce::dontSendNotification);
            int id = 1;
            for (auto& n : mgr.getNetworks())
                ssidBox.addItem (n.ssid + (n.lock ? "" : "  (open)"), id++);
            ssidBox.setText (mgr.get ("wifi.ssid").toString(), juce::dontSendNotification);
        }
        scanButton.setButtonText (mgr.isScanningWifi() ? "..." : "SCAN");
        passEditor.setTextToShowWhenEmpty ((bool) mgr.get ("wifi.hasPass") ? "saved - type to change" : "none", Pequod::Colours::textDim);
        pass2Editor.setTextToShowWhenEmpty ((bool) mgr.get ("wifi.hasPass2") ? "saved - type to change" : "none", Pequod::Colours::textDim);
        setIfIdle (ssid2Editor, mgr.get ("wifi.ssid2").toString());
        setIfIdle (hostEditor, mgr.get ("wifi.hostname").toString());
        setIfIdle (ipEditor, mgr.getManualAddress());
        for (auto* c : std::initializer_list<juce::Component*> { &ssidBox, &scanButton, &passEditor, &ssid2Editor, &pass2Editor, &hostEditor })
            c->setEnabled (wifiOn);

        auto& i = mgr.getInfo();
        juce::String w;
        if (! wifiOn)                                   w = "Off";
        else if (i.wifiConnected)                       w = "Connected to " + i.ssid + "  -  " + i.ip;
        else if (mgr.get ("wifi.ssid").toString().isEmpty()) w = "Not set up - choose a network and type its password";
        else                                            w = "Trying to connect to " + mgr.get ("wifi.ssid").toString() + "...";
        wifiStatus.setText (w, juce::dontSendNotification);

        info.setText ("Firmware " + i.fw + "   -   connected by " + mgr.describeConnection() + "\nID " + i.id
                      + "   -   " + juce::String ((int) i.units.size()) + " board" + (i.units.size() == 1 ? "" : "s"),
                      juce::dontSendNotification);
        updateLabel.setText (mgr.isUpdating() ? mgr.updateStatus() : mgr.updateStatus().isNotEmpty() ? mgr.updateStatus()
                             : mgr.newerFirmwareAvailable() ? "Firmware " + ControllerManager::bundledFirmwareVersion() + " came with this PEQUOD - press UPDATE FIRMWARE"
                             : juce::String ("Load a new PEQUOD_FC.bin onto the controller"), juce::dontSendNotification);
        updateLabel.setColour (juce::Label::textColourId, mgr.newerFirmwareAvailable() ? Pequod::Colours::mustard : Pequod::Colours::textDim);
        updateButton.setEnabled (! mgr.isUpdating());
    }

    void paint (juce::Graphics& g) override { form.paint (g); }
    void resized() override { form.layout (juce::Rectangle<int> (24, 8, juce::jmax (300, getWidth() - 48), 100)); }

private:
    ControllerManager& mgr;
    Form form;
    juce::TextEditor nameEditor, passEditor, ssid2Editor, pass2Editor, hostEditor, ipEditor;
    juce::Slider brightness, offGlow, contrast;
    juce::ComboBox layersBox, ssidBox;
    juce::ToggleButton tunerToggle, usbToggle, dinToggle, wifiToggle;
    juce::TextButton renameButton { "RENAME" }, scanButton { "SCAN" }, rescanButton { "FIND BOARDS AGAIN" }, rebootButton { "RESTART" },
                     updateButton { "UPDATE FIRMWARE" }, factoryButton { "FACTORY RESET" };
    juce::OwnedArray<juce::TextButton> layerColourButtons;
    juce::Label wifiStatus, info, updateLabel;
    std::unique_ptr<juce::FileChooser> chooser;

    void commitName()
    {
        nameEditor.unfocusAllComponents();
        mgr.setName (nameEditor.getText());
    }

    void commitPassword (juce::TextEditor& e, const juce::String& path)
    {
        if (e.getText().isEmpty()) return;      // empty = keep the saved one
        mgr.set (path, e.getText());
        e.clear();
        if (mgr.onStatus) mgr.onStatus ("Wi-Fi password sent to the controller");
    }

    void pickLayerColour (int layerIndex)
    {
        juce::PopupMenu menu;
        for (auto c : swatchColours)
        {
            const auto col = juce::Colour (0xff000000 | c);
            juce::PopupMenu::Item item (juce::String::repeatedString (juce::String::charToString ((juce::juce_wchar) 0x25CF), 3) + "  " + hex (col));
            item.setColour (col).setAction ([this, layerIndex, col] { mgr.set ("layerColors." + juce::String (layerIndex), hex (col)); });
            menu.addItem (item);
        }
        menu.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (layerColourButtons[layerIndex]));
    }

    void chooseFirmware()
    {
        // The Windows download ships the matching firmware next to PEQUOD Host.exe.
        auto bundled = ControllerManager::bundledFirmwareFolder();
        auto start = bundled.getChildFile ("PEQUOD_FC.bin").existsAsFile() ? bundled.getChildFile ("PEQUOD_FC.bin") : juce::File();
        chooser = std::make_unique<juce::FileChooser> ("Choose the controller firmware (PEQUOD_FC.bin)", start, "*.bin");
        chooser->launchAsync (juce::FileBrowserComponent::openMode | juce::FileBrowserComponent::canSelectFiles,
                              [this] (const juce::FileChooser& fc)
        {
            auto f = fc.getResult();
            if (! f.existsAsFile()) return;
            juce::String why;
            if (! mgr.startFirmwareUpdate (f, why))
                juce::AlertWindow::showMessageBoxAsync (juce::MessageBoxIconType::WarningIcon, "Firmware update", why);
        });
    }

    void confirmFactoryReset()
    {
        juce::AlertWindow::showOkCancelBox (juce::MessageBoxIconType::WarningIcon, "Factory reset",
            "Erase every setting on the controller (switch functions, colours, Wi-Fi, calibration)?\n"
            "Its name is kept. PEQUOD's own assignments stay as they are.",
            "Erase", "Cancel", nullptr,
            juce::ModalCallbackFunction::create ([this] (int r) { if (r == 1) mgr.factoryReset(); }));
    }
};

//==============================================================================
ControllerPanel::ControllerPanel (ControllerManager& m, HostEngine& e, ControlSurface& c)
    : mgr (m), engine (e), control (c)
{
    board = std::make_unique<BoardView> (mgr);
    inspector = std::make_unique<Inspector> (mgr, engine, control);
    settings = std::make_unique<SettingsPage> (mgr);

    boardViewport.setViewedComponent (board.get(), false);
    boardViewport.setScrollBarsShown (false, true);
    controlsPage.addAndMakeVisible (boardViewport);
    controlsPage.addAndMakeVisible (*inspector);
    controlsViewport.setViewedComponent (&controlsPage, false);
    controlsViewport.setScrollBarsShown (true, false);
    addChildComponent (controlsViewport);
    settingsViewport.setViewedComponent (settings.get(), false);
    settingsViewport.setScrollBarsShown (true, false);
    addChildComponent (settingsViewport);

    for (auto* b : { &controlsTab, &settingsTab })
    {
        b->setClickingTogglesState (false);
        b->setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
        b->setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
        addChildComponent (b);
    }
    controlsTab.onClick = [this] { showSettings (false); };
    settingsTab.onClick = [this] { showSettings (true); };


    for (int i = 0; i < 4; ++i)
    {
        auto* b = layerButtons.add (new juce::TextButton ("LAYER " + juce::String (i + 1)));
        b->setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
        b->setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
        b->onClick = [this, i] { mgr.selectLayer (i); };
        addChildComponent (b);
    }

    board->onSelect = [this] (const juce::String& uid, int code) { select (uid, code); };
    inspector->onLayoutChanged = [this] { if (! inResized) resized(); };
    mgr.onControlPressed = [this] (const juce::String& uid, int code)
    {
        if (isShowing()) { showSettings (false); select (uid, code); }
    };

    styleEditor (ipEditor);
    ipEditor.setTextToShowWhenEmpty ("controller IP, e.g. 192.168.1.40", Pequod::Colours::textDim);
    ipEditor.setText (mgr.getManualAddress(), false);
    ipEditor.onReturnKey = [this] { mgr.setManualAddress (ipEditor.getText()); };
    ipButton.onClick = [this] { mgr.setManualAddress (ipEditor.getText()); };
    addChildComponent (ipEditor);
    addChildComponent (ipButton);

    mgr.addChangeListener (this);
    control.addChangeListener (this);
    updateVisibility();
    changeListenerCallback (nullptr);
    startTimerHz (30);
}

ControllerPanel::~ControllerPanel()
{
    stopTimer();
    mgr.onControlPressed = nullptr;
    control.removeChangeListener (this);
    mgr.removeChangeListener (this);
}

void ControllerPanel::select (const juce::String& uid, int code)
{
    board->selUid = uid;
    board->selCode = code;
    inspector->setControl (uid, code);
    board->repaint();
}

void ControllerPanel::showSettings (bool s)
{
    showingSettings = s;
    updateVisibility();
}

void ControllerPanel::updateVisibility()
{
    const bool conn = mgr.isConnected();
    controlsViewport.setVisible (conn && ! showingSettings);
    settingsViewport.setVisible (conn && showingSettings);
    controlsTab.setVisible (conn);
    settingsTab.setVisible (conn);
    controlsTab.setToggleState (! showingSettings, juce::dontSendNotification);
    settingsTab.setToggleState (showingSettings, juce::dontSendNotification);
    for (int i = 0; i < layerButtons.size(); ++i)
    {
        layerButtons[i]->setVisible (conn && ! showingSettings && mgr.getNumLayers() > 1 && i < mgr.getNumLayers());
        layerButtons[i]->setToggleState (i == mgr.getLayer(), juce::dontSendNotification);
    }
    ipEditor.setVisible (! conn);
    ipButton.setVisible (! conn);
    resized();
    repaint();
}

void ControllerPanel::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::bg);

    auto top = getLocalBounds().reduced (18, 12).removeFromTop (44);
    if (! mgr.isConnected())
    {
        auto r = getLocalBounds().reduced (60, 40).withTrimmedBottom (90);
        g.setColour (Pequod::Colours::mustard);
        g.setFont (Pequod::heading (26.0f));
        g.drawText ("NO CONTROLLER CONNECTED", r.removeFromTop (r.getHeight() / 3).withTrimmedBottom (10), juce::Justification::centredBottom);
        g.setColour (Pequod::Colours::text);
        g.setFont (Pequod::body (16.0f));
        g.drawFittedText ("Plug the PEQUOD FC into this computer with USB - PEQUOD finds it by itself.\n\n"
                          "Or use it over Wi-Fi: once it's plugged in, pick your network under SETTINGS. "
                          "After that, it connects whenever this computer and the controller share that network.\n\n"
                          "Still on the old controller firmware? Flash it with the new firmware first - see README in the \"Controller firmware\" folder next to PEQUOD Host.",
                          r.reduced (40, 0), juce::Justification::centredTop, 10);
        auto tried = mgr.getLink().describeUsbPorts();
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::body (13.0f));
        g.drawFittedText (tried.isEmpty() ? juce::String ("No USB MIDI devices found on this computer.")
                                          : "USB MIDI devices checked:   " + tried.joinIntoString ("     "),
                          ipEditor.getBounds().translated (0, -80).withHeight (44).withSizeKeepingCentre (getWidth() - 120, 44),
                          juce::Justification::centred, 3);
        g.drawText ("Wi-Fi network blocks discovery? Type the controller's IP:", ipEditor.getBounds().translated (0, -26).withWidth (500),
                    juce::Justification::centredLeft);
        return;
    }

    // status dot + name + connection
    auto& info = mgr.getInfo();
    g.setColour (juce::Colour (0xff4caf50));
    g.fillEllipse (juce::Rectangle<float> (12.0f, 12.0f).withCentre ({ (float) top.getX() + 6.0f, (float) top.getCentreY() }));
    g.setColour (Pequod::Colours::mustard);
    g.setFont (Pequod::heading (22.0f));
    auto nameArea = top.withTrimmedLeft (20);
    const auto name = (mgr.hasConfig() ? mgr.get ("name").toString() : info.name).toUpperCase();
    g.drawText (name, nameArea, juce::Justification::centredLeft);
    const int nameW = juce::GlyphArrangement::getStringWidthInt (Pequod::heading (22.0f), name);
    g.setColour (Pequod::Colours::textDim);
    g.setFont (Pequod::body (13.0f));
    g.drawText (mgr.describeConnection() + "   -   firmware " + info.fw, nameArea.withTrimmedLeft (nameW + 16), juce::Justification::centredLeft);

    if (! showingSettings)
    {
        auto bar = barArea.withLeft (layerButtons[0]->isVisible() ? layerButtons[mgr.getNumLayers() - 1]->getRight() + 16 : barArea.getX());
        auto bank = control.getBankName ? control.getBankName() : juce::String();
        auto preset = control.getPresetName ? control.getPresetName() : juce::String();
        g.setFont (Pequod::heading (12.0f, false));
        g.setColour (Pequod::Colours::textDim);
        g.drawText ("EDITING PRESET", bar.removeFromLeft (120), juce::Justification::centredLeft);
        g.setFont (Pequod::heading (15.0f));
        g.setColour (Pequod::Colours::mustard);
        g.drawText ((bank.isNotEmpty() ? bank + "  /  " : juce::String()) + preset, bar, juce::Justification::centredLeft, true);
    }
}

void ControllerPanel::resized()
{
    const juce::ScopedValueSetter<bool> busy (inResized, true);
    auto r = getLocalBounds().reduced (18, 12);
    auto top = r.removeFromTop (44);

    if (! mgr.isConnected())
    {
        auto bottom = getLocalBounds().reduced (60, 40).removeFromBottom (60);
        auto row = bottom.withSizeKeepingCentre (500, 32);
        ipButton.setBounds (row.removeFromRight (100));
        row.removeFromRight (8);
        ipEditor.setBounds (row);
        return;
    }

    settingsTab.setBounds (top.removeFromRight (100).withSizeKeepingCentre (100, 30));
    top.removeFromRight (4);
    controlsTab.setBounds (top.removeFromRight (100).withSizeKeepingCentre (100, 30));

    r.removeFromTop (10);
    settingsViewport.setBounds (r);
    settings->setSize (r.getWidth() - settingsViewport.getScrollBarThickness(), 10);
    settings->setSize (settings->getWidth(), settings->getIdealHeight());

    auto bar = r.removeFromTop (32);
    barArea = bar;
    for (auto* b : layerButtons)
        if (b->isVisible()) { b->setBounds (bar.removeFromLeft (84)); bar.removeFromLeft (4); }
    r.removeFromTop (10);

    // boards across the full width (scroll sideways if there are many), settings for the picked control below
    // Everything below scrolls when the window is smaller than it needs, so nothing is ever cut off.
    controlsViewport.setBounds (r);
    const int pageW = r.getWidth() - (controlsViewport.canScrollVertically() ? controlsViewport.getScrollBarThickness() : 0);
    const int boardH = BoardView::getRequiredHeight() + boardViewport.getScrollBarThickness();
    const int inspectorH = juce::jmax (r.getHeight() - boardH - 10, inspector->getIdealHeight (pageW));
    const bool needsScroll = boardH + 10 + inspectorH > r.getHeight();
    const int w = needsScroll ? r.getWidth() - controlsViewport.getScrollBarThickness() : r.getWidth();
    controlsPage.setSize (w, boardH + 10 + juce::jmax (r.getHeight() - boardH - 10, inspector->getIdealHeight (w)));
    boardViewport.setBounds (0, 0, w, boardH);
    board->setSize (juce::jmax (w, board->getRequiredWidth()), BoardView::getRequiredHeight());
    inspector->setBounds (0, boardH + 10, w, controlsPage.getHeight() - boardH - 10);
}

void ControllerPanel::changeListenerCallback (juce::ChangeBroadcaster*)
{
    // Something on the controller or in PEQUOD changed: redraw from the manager.
    juce::String sig;
    sig << (int) mgr.isConnected() << mgr.getNumLayers() << mgr.getLayer() << (int) mgr.hasConfig() << (int) mgr.getInfo().units.size();
    if (sig != lastSignature) { lastSignature = sig; updateVisibility(); }

    if (mgr.isConnected() && inspector->getUid().isNotEmpty() && mgr.getInfo().findUnit (inspector->getUid()) == nullptr)
        select ({}, -1);
    if (mgr.isConnected() && inspector->getUid().isEmpty() && ! mgr.getInfo().units.empty() && mgr.hasConfig())
    {
        auto& u = mgr.getInfo().units.front();
        select (u.uid, ControllerManager::controlCodes (u)[0]);
    }

    inspector->refresh();
    settings->refresh();
    board->repaint();
}

void ControllerPanel::timerCallback()
{
    if (mgr.isConnected() != wasConnected)
    {
        wasConnected = mgr.isConnected();
        updateVisibility();
    }
    if (! isShowing()) return;
    if (++idleTicks % 15 == 0) repaint();         // checked ports / which preset is being edited
    if (! mgr.isConnected()) return;
    inspector->tick();
    board->repaint();
}

//==============================================================================
ControllerWindow::ControllerWindow (ControllerManager& m, HostEngine& e, ControlSurface& c)
    : DocumentWindow ("PEQUOD - Foot controller", Pequod::Colours::bg, juce::DocumentWindow::closeButton | juce::DocumentWindow::minimiseButton)
{
    setUsingNativeTitleBar (true);
    panel = new ControllerPanel (m, e, c);
    panel->setSize (1360, 900);
    setContentOwned (panel, true);
    setResizable (true, false);
    setResizeLimits (560, 420, 2600, 1800);
    centreWithSize (getWidth(), getHeight());
}
