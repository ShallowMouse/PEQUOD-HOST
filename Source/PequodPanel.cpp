#include "PequodPanel.h"
#include <BinaryData.h>

// A port of the VST3 panel (engine/vintage/src/ui.rs): keep the two in step.
namespace
{
    using juce::Colour;
    using Pt = juce::Point<float>;
    using Rect = juce::Rectangle<float>;

    const Colour WOOD        { 0xff3a2416 };
    const Colour WOOD_DARK   { 0xff24160d };
    const Colour PLATE       { 0xffe8ddc4 };
    const Colour PLATE_EDGE  { 0xffb9a986 };
    const Colour INK         { 0xff2b2118 };
    const Colour INK_SOFT    { 0xff6b5a48 };
    const Colour CREAM       { 0xfff6eedc };
    const Colour BAKELITE    { 0xff1c1714 };
    const Colour BAKELITE_HI { 0xff332b25 };
    const Colour METER_FACE  { 0xfff3e2a6 };

    constexpr float pi = juce::MathConstants<float>::pi;

    Colour blackAlpha (int a) { return Colour ((juce::uint8) 0, (juce::uint8) 0, (juce::uint8) 0, (juce::uint8) juce::jlimit (0, 255, a)); }
    Colour lerp (Colour a, Colour b, float t) { return a.interpolatedWith (b, juce::jlimit (0.0f, 1.0f, t)); }

    // egui's polar(): angle 0 points up, clockwise (the same as JUCE's getPointOnCircumference)
    Pt polar (Pt c, float r, float a) { return { c.x + r * std::sin (a), c.y - r * std::cos (a) }; }
    float knobAngle (float v) { return (-135.0f + 270.0f * juce::jlimit (0.0f, 1.0f, v)) * pi / 180.0f; }

    float f (const juce::var& v, float d = 0) { return v.isVoid() ? d : (float) (double) v; }
    int i (const juce::var& v, int d = -1) { return v.isVoid() ? d : (int) v; }

    juce::StringArray strings (const juce::var& v)
    {
        juce::StringArray s;
        if (auto* a = v.getArray())
            for (auto& x : *a) s.add (x.toString());
        return s;
    }

    // The VST3 panel's fonts: egui's defaults (Ubuntu Light, and Hack for the mark), at the same em size.
    juce::Typeface::Ptr typeface (bool mono)
    {
        static auto ubuntu = juce::Typeface::createSystemTypefaceFor (BinaryData::UbuntuLight_ttf, (size_t) BinaryData::UbuntuLight_ttfSize);
        static auto hack   = juce::Typeface::createSystemTypefaceFor (BinaryData::HackRegular_ttf, (size_t) BinaryData::HackRegular_ttfSize);
        return mono ? hack : ubuntu;
    }

    juce::Font font (float size, bool mono = false)
    {
        return juce::Font (juce::FontOptions (typeface (mono)).withPointHeight (size));
    }

    /** egui's painter.text(): `align` -1/0/1 per axis says which point of the text sits at `at`. */
    void say (juce::Graphics& g, const juce::String& s, Pt at, int ax, int ay, float size, Colour c, bool mono = false)
    {
        auto fnt = font (size, mono);
        g.setFont (fnt);
        g.setColour (c);
        const float w = juce::GlyphArrangement::getStringWidth (fnt, s) + 2.0f;
        const float h = fnt.getHeight();
        const float x = ax < 0 ? at.x : ax > 0 ? at.x - w : at.x - w * 0.5f;
        const float y = ay < 0 ? at.y : ay > 0 ? at.y - h : at.y - h * 0.5f;
        g.drawText (s, Rect (x, y, w, h), ax < 0 ? juce::Justification::centredLeft : ax > 0 ? juce::Justification::centredRight : juce::Justification::centred, false);
    }

    void line (juce::Graphics& g, Pt a, Pt b, float width, Colour c)
    {
        g.setColour (c);
        g.drawLine ({ a, b }, width);
    }

    void circle (juce::Graphics& g, Pt c, float r, Colour col)
    {
        g.setColour (col);
        g.fillEllipse (c.x - r, c.y - r, r * 2.0f, r * 2.0f);
    }

    void ring (juce::Graphics& g, Pt c, float r, float width, Colour col)
    {
        g.setColour (col);
        g.drawEllipse (c.x - r, c.y - r, r * 2.0f, r * 2.0f, width);
    }

    template <typename Fn>
    void polyline (juce::Graphics& g, int n, Fn&& point, float width, Colour c)
    {
        juce::Path p;
        for (int k = 0; k <= n; ++k)
        {
            auto q = point (k);
            if (k == 0) p.startNewSubPath (q); else p.lineTo (q);
        }
        g.setColour (c);
        g.strokePath (p, juce::PathStrokeType (width, juce::PathStrokeType::curved, juce::PathStrokeType::rounded));
    }

    float vuDb (float peak)
    {
        // 0 VU = -18 dBFS; display range -20..+3
        return juce::jlimit (-25.0f, 4.0f, 20.0f * std::log10 (juce::jmax (peak, 1.0e-6f)) + 18.0f);
    }

    const char* noteNames[] = { "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B" };
}

//==============================================================================
PequodPanel::PequodPanel (PequodNative::Instance& p)
    : juce::AudioProcessorEditor (p), inst (p)
{
    const auto& j = PequodNative::panelOf (inst.info());
    title = j["title"].toString();
    if (auto* s = j["subtitle"].getArray(); s != nullptr && s->size() >= 2)
    {
        sub1 = (*s)[0].toString();
        sub2 = (*s)[1].toString();
    }
    accent = Colour::fromString ("ff" + j["accent"].toString().removeCharacters ("#"));
    panelW = juce::jmax (200, i (j["w"], 640));
    panelH = juce::jmax (120, i (j["h"], 432));

    if (auto* arr = j["items"].getArray())
        for (auto& v : *arr)
        {
            Item it;
            const auto t = v["type"].toString();
            it.kind = t == "knob" ? Kind::knob : t == "switch" ? Kind::sw : t == "selector" ? Kind::selector
                    : t == "meter" ? Kind::meter : t == "lamp" ? Kind::lamp : t == "divider" ? Kind::divider
                    : t == "text" ? Kind::text : t == "radio" ? Kind::radio : t == "caption" ? Kind::caption
                    : t == "roomplan" ? Kind::roomplan : t == "polar" ? Kind::polar : t == "irloader" ? Kind::irloader
                    : t == "tuner" ? Kind::tuner : t == "rotor" ? Kind::rotor : Kind::unknown;
            it.p = i (v["p"]);
            it.m = i (v["m"]);
            it.x = f (v["x"]); it.y = f (v["y"]); it.r = f (v["r"], 20); it.w = f (v["w"]); it.h = f (v["h"]);
            it.size = f (v["size"], 12); it.lo = f (v["lo"]); it.hi = f (v["hi"], 1); it.rowH = f (v["row_h"], 22);
            it.label = v["label"].toString().isNotEmpty() ? v["label"].toString() : v["caption"].toString();
            if (it.kind == Kind::text) it.label = v["text"].toString();
            if (auto* a = v["align"].getArray(); a != nullptr && a->size() == 2)
            {
                it.alignX = (int) (*a)[0];
                it.alignY = (int) (*a)[1];
            }
            it.tip = v["tip"].toString();
            it.meterKind = v["kind"].toString();
            if (it.kind == Kind::sw)            it.labels = strings (v["labels"]);
            else if (it.kind == Kind::selector) it.labels = strings (v["positions"]);
            else if (it.kind == Kind::radio)    it.labels = strings (v["names"]);
            else if (it.kind == Kind::caption)  it.labels = strings (v["texts"]);
            if (auto* a = v["active"].getArray(); a != nullptr && a->size() == 3)
            {
                it.activeP = (int) (*a)[0];
                it.activeMin = (float) (double) (*a)[1];
                it.activeMax = (float) (double) (*a)[2];
            }
            it.room = i (v["room"]); it.distance = i (v["distance"]); it.spacing = i (v["spacing"]);
            it.pattern = i (v["pattern"]); it.angle = i (v["angle"]);
            it.freq = i (v["freq"]); it.level = i (v["level"]); it.reference = i (v["reference"]);
            it.horn = i (v["horn"]); it.drum = i (v["drum"]);
            it.cab = i (v["cab"]); it.irPos = i (v["ir_pos"]);
            if (auto* ps = v["plans"].getArray())
                for (auto& pv : *ps)
                {
                    Plan pl;
                    pl.length = f (pv["length"], 10);
                    pl.width = f (pv["width"], 6);
                    if (auto* s = pv["src"].getArray(); s != nullptr && s->size() == 2)
                    {
                        pl.srcX = (float) (double) (*s)[0];
                        pl.srcY = (float) (double) (*s)[1];
                    }
                    pl.openAir = (bool) pv["open_air"];
                    pl.farWall = pv["far_wall"].toString();
                    it.plans.push_back (pl);
                }
            items.push_back (std::move (it));
        }

    if (auto* arr = j["presets"].getArray())
        for (auto& v : *arr)
        {
            Preset pr;
            pr.name = v["name"].toString();
            if (auto* vals = v["values"].getArray())
                for (auto& pair : *vals)
                    if (auto* a = pair.getArray(); a != nullptr && a->size() == 2)
                        pr.values.push_back ({ (int) (*a)[0], (float) (double) (*a)[1] });
            presets.push_back (pr);
        }

    raw.assign ((size_t) juce::jmax (1, inst.info().meters), 0.0f);
    shown = raw;
    for (auto& it : items)   // VU needles start at rest
        if (it.kind == Kind::meter && it.meterKind != "gr" && juce::isPositiveAndBelow (it.m, (int) shown.size()))
            shown[(size_t) it.m] = vuDb (0.0f);
    lastValues.assign ((size_t) inst.getParameters().size(), -1.0f);
    lastTick = juce::Time::getMillisecondCounterHiRes() * 0.001;

    setSize (panelW, panelH);
    setWantsKeyboardFocus (false);
    startTimerHz (30);
}

PequodPanel::~PequodPanel()
{
    stopTimer();
    // a hosted plugin's editor tells its processor it's gone (as JUCE's VST3 editors do),
    // otherwise getActiveEditor() keeps pointing at it and the panel can't be opened again
    processor.editorBeingDeleted (this);
}

//==============================================================================
juce::AudioProcessorParameter* PequodPanel::paramObj (int p) const
{
    auto& ps = inst.getParameters();
    return juce::isPositiveAndBelow (p, ps.size()) ? ps[p] : nullptr;
}

float PequodPanel::value (int p) const
{
    if (auto* o = paramObj (p)) return o->getValue();
    return 0.0f;
}

float PequodPanel::plain (int p) const
{
    return paramObj (p) != nullptr ? inst.plain (p, value (p)) : 0.0f;
}

int PequodPanel::stepsOf (int p) const
{
    // as the VST3 panel: continuous parameters count as one step
    auto& ps = inst.info().params;
    return juce::isPositiveAndBelow (p, (int) ps.size()) ? juce::jmax (1, ps[(size_t) p].steps) : 1;
}

int PequodPanel::indexOf (int p) const
{
    const int s = stepsOf (p);
    return juce::jlimit (0, s, juce::roundToInt (value (p) * (float) s));
}

void PequodPanel::setValue (int p, float v, bool gesture)
{
    if (auto* o = paramObj (p))
    {
        v = juce::jlimit (0.0f, 1.0f, v);
        if (gesture) o->beginChangeGesture();
        if (std::abs (o->getValue() - v) > 1.0e-7f)
            o->setValueNotifyingHost (v);
        if (gesture) o->endChangeGesture();
        edited();
    }
}

void PequodPanel::setIndex (int p, int index)
{
    const int s = stepsOf (p);
    setValue (p, (float) juce::jlimit (0, s, index) / (float) s, true);
}

void PequodPanel::edited()
{
    if (currentPreset >= 0) presetDirty = true;
    repaint();
}

bool PequodPanel::isDim (const Item& it) const
{
    if (it.activeP < 0) return false;
    const float v = value (it.activeP);
    return ! (v >= it.activeMin - 1.0e-4f && v <= it.activeMax + 1.0e-4f);
}

juce::String PequodPanel::text (int p) const
{
    if (auto* o = paramObj (p)) return o->getText (o->getValue(), 64);
    return {};
}

//==============================================================================
void PequodPanel::paint (juce::Graphics& g)
{
    drawFaceplate (g);

    for (int k = 0; k < (int) items.size(); ++k)
    {
        auto& it = items[(size_t) k];
        switch (it.kind)
        {
            case Kind::knob:     drawKnob (g, it, k); break;
            case Kind::sw:       drawSwitch (g, it, k); break;
            case Kind::selector: drawSelector (g, it, k); break;
            case Kind::meter:    drawMeter (g, it); break;
            case Kind::lamp:
                drawLamp (g, it);
                say (g, it.label, { it.x + 14.0f, it.y }, -1, 0, 12.0f, INK);
                break;
            case Kind::rotor:    drawRotor (g, it); break;
            case Kind::divider:  line (g, { 40.0f, it.y }, { (float) panelW - 40.0f, it.y }, 1.0f, PLATE_EDGE); break;
            case Kind::text:     say (g, it.label, { it.x, it.y }, it.alignX, it.alignY, it.size, INK); break;
            case Kind::radio:    drawRadio (g, it); break;
            case Kind::caption:  drawCaption (g, it); break;
            case Kind::roomplan: drawRoomPlan (g, it); break;
            case Kind::polar:    drawPolar (g, it); break;
            case Kind::irloader: drawIrLoader (g, it); break;
            case Kind::tuner:    drawTuner (g, it); break;
            case Kind::unknown:  break;
        }
    }

    const float h = (float) panelH;
    say (g, "[PEQUOD]", { 38.0f, h - 36.0f }, -1, 0, 12.0f, Colour (0xffa8823a), true);
    if (! presets.empty())
    {
        drawPresetBar (g);
        if (presetOpen) drawPresetList (g);
    }
}

void PequodPanel::drawFaceplate (juce::Graphics& g) const
{
    const float w = (float) panelW, h = (float) panelH;
    g.fillAll (WOOD);
    for (int k = 0; (float) k * 29.0f < h; ++k)
    {
        const float y = 6.0f + (float) k * 29.0f;
        line (g, { 0.0f, y }, { (float) getWidth(), y + 3.0f }, 1.0f, WOOD_DARK);
    }
    const Rect plate (14.0f, 14.0f, w - 28.0f, h - 28.0f);
    g.setColour (blackAlpha (90));
    g.fillRoundedRectangle (plate.translated (0.0f, 3.0f), 10.0f);
    g.setColour (PLATE);
    g.fillRoundedRectangle (plate, 10.0f);
    g.setColour (PLATE_EDGE);
    g.drawRoundedRectangle (plate.reduced (1.0f), 9.0f, 2.0f);

    juce::Path band;
    band.addRoundedRectangle (14.0f, 14.0f, w - 28.0f, 56.0f, 10.0f, 10.0f, true, true, false, false);
    g.setColour (accent);
    g.fillPath (band);
    say (g, title, { 44.0f, 42.0f }, -1, 0, 30.0f, CREAM);
    say (g, sub1, { w - 44.0f, 36.0f }, 1, 0, 12.0f, CREAM);
    say (g, sub2, { w - 44.0f, 52.0f }, 1, 0, 12.0f, CREAM);

    for (auto c : { Pt (28.0f, 84.0f), Pt (w - 28.0f, 84.0f), Pt (28.0f, h - 28.0f), Pt (w - 28.0f, h - 28.0f) })
    {
        circle (g, c, 6.0f, PLATE_EDGE);
        circle (g, c, 4.8f, Colour (0xffcfc4ab));
        line (g, c - Pt (3.5f, 1.0f), c + Pt (3.5f, 1.0f), 1.4f, INK_SOFT);
    }
}

//==============================================================================
void PequodPanel::drawKnob (juce::Graphics& g, const Item& it, int index) const
{
    const bool dim = isDim (it);
    const bool big = it.r > 30.0f;
    const Pt c (it.x, it.y);
    const float r = it.r;
    auto fade = [dim] (Colour col) { return dim ? lerp (col, PLATE, 0.6f) : col; };
    const auto ink = fade (INK), inkSoft = fade (INK_SOFT), acc = fade (accent), bake = fade (BAKELITE), bakeHi = fade (BAKELITE_HI), cream = fade (CREAM);
    const float v = value (it.p);

    const int ticks = big ? 11 : 7;
    for (int t = 0; t < ticks; ++t)
    {
        const float a = knobAngle ((float) t / (float) (ticks - 1));
        const bool lng = t == 0 || t == ticks - 1 || (big && t == ticks / 2);
        line (g, polar (c, r + 5.0f, a), polar (c, r + (lng ? 12.0f : 9.0f), a), lng ? 2.0f : 1.2f, ink);
    }
    const float a0 = knobAngle (0.0f), a1 = knobAngle (v);
    if (v > 0.001f)
        polyline (g, 40, [&] (int k) { return polar (c, r + 2.5f, a0 + (a1 - a0) * (float) k / 40.0f); }, 3.0f, acc);
    circle (g, c + Pt (0.0f, 3.0f), r, blackAlpha (dim ? 25 : 80));
    circle (g, c, r, bake);
    circle (g, c, r * 0.82f, bakeHi);
    ring (g, c, r * 0.82f, 1.0f, bake);
    for (int k = 0; k < 24; ++k)
    {
        const float a = (float) k * pi / 12.0f + a1;
        line (g, polar (c, r * 0.86f, a), polar (c, r * 0.98f, a), 1.0f, bakeHi);
    }
    line (g, polar (c, r * 0.2f, a1), polar (c, r * 0.78f, a1), big ? 4.0f : 3.0f, cream);
    if (hoverItem == index || dragItem == index)
        ring (g, c, r + 1.0f, 1.5f, accent);
    const float lf = big ? 16.0f : 11.0f, vf = big ? 13.0f : 11.0f;
    say (g, it.label, c - Pt (0.0f, r + (big ? 22.0f : 20.0f)), 0, 0, lf, ink);
    say (g, text (it.p), c + Pt (0.0f, r + (big ? 22.0f : 18.0f)), 0, 0, vf, inkSoft);
}

void PequodPanel::drawSwitch (juce::Graphics& g, const Item& it, int index) const
{
    const bool on = value (it.p) > 0.5f;
    const Pt c (it.x, it.y);
    const Rect pill (c.x - 17.0f, c.y - 8.0f, 34.0f, 16.0f);
    if (it.labels.size() >= 2)
    {
        say (g, it.labels[0], { pill.getX() - 6.0f, c.y }, 1, 0, 11.0f, on ? INK_SOFT : INK);
        say (g, it.labels[1], { pill.getRight() + 6.0f, c.y }, -1, 0, 11.0f, on ? INK : INK_SOFT);
    }
    g.setColour (BAKELITE);
    g.fillRoundedRectangle (pill, 8.0f);
    if (on)
    {
        g.setColour (accent);
        g.fillRoundedRectangle (pill.reduced (2.0f), 6.0f);
    }
    const Pt kc (on ? pill.getRight() - 8.0f : pill.getX() + 8.0f, c.y);
    circle (g, kc + Pt (0.0f, 1.0f), 6.5f, blackAlpha (90));
    circle (g, kc, 6.0f, CREAM);
    if (hoverItem == index)
    {
        g.setColour (accent);
        g.drawRoundedRectangle (pill.expanded (2.6f), 9.0f, 1.2f);
    }
}

float PequodPanel::selectorAngle (const Item& it, int k) const
{
    // positions spread over an arc above the knob
    const int n = stepsOf (it.p) + 1;
    const float span = n <= 3 ? 100.0f : n == 4 ? 150.0f : 200.0f;
    return (-span / 2.0f + span * (float) k / (float) juce::jmax (1, n - 1)) * pi / 180.0f;
}

void PequodPanel::drawSelector (juce::Graphics& g, const Item& it, int index) const
{
    const int n = stepsOf (it.p) + 1;
    const float r = 20.0f;
    const Pt c (it.x, it.y);
    const int current = indexOf (it.p);
    for (int k = 0; k < n; ++k)
    {
        const float a = selectorAngle (it, k);
        line (g, polar (c, r + 4.0f, a), polar (c, r + 9.0f, a), 1.5f, INK);
        if (k >= it.labels.size()) continue;
        const auto col = k == current ? accent : INK_SOFT;
        // side labels hang outward so they clear the knob
        if (n >= 4 && std::sin (a) > 0.6f)       say (g, it.labels[k], polar (c, r + 12.0f, a), -1, 0, 10.0f, col);
        else if (n >= 4 && std::sin (a) < -0.6f) say (g, it.labels[k], polar (c, r + 12.0f, a), 1, 0, 10.0f, col);
        else                                     say (g, it.labels[k], polar (c, r + 20.0f, a), 0, 0, 10.0f, col);
    }
    // chicken-head knob
    const float a = selectorAngle (it, current);
    circle (g, c + Pt (0.0f, 3.0f), r, blackAlpha (80));
    circle (g, c, r * 0.9f, BAKELITE);
    juce::Path head;
    head.startNewSubPath (polar (c, r * 1.05f, a));
    head.lineTo (polar (c, r * 0.45f, a + pi / 2.0f));
    head.lineTo (polar (c, r * 0.65f, a + pi));
    head.lineTo (polar (c, r * 0.45f, a - pi / 2.0f));
    head.closeSubPath();
    g.setColour (BAKELITE_HI);
    g.fillPath (head);
    g.setColour (BAKELITE);
    g.strokePath (head, juce::PathStrokeType (1.0f));
    line (g, c, polar (c, r * 0.95f, a), 2.5f, CREAM);
    if (hoverItem == index || dragItem == index)
        ring (g, c, r + 1.0f, 1.5f, accent);
    say (g, it.label, c + Pt (0.0f, r + 13.0f), 0, 0, 11.0f, INK);
}

void PequodPanel::drawMeter (juce::Graphics& g, const Item& it) const
{
    const Rect rect (it.x, it.y, it.w, it.h);
    const bool gr = it.meterKind == "gr";
    const float value = juce::isPositiveAndBelow (it.m, (int) shown.size()) ? shown[(size_t) it.m] : (gr ? 0.0f : -25.0f);
    g.setColour (BAKELITE);
    g.fillRoundedRectangle (rect.expanded (5.0f), 6.0f);
    g.setColour (METER_FACE);
    g.fillRoundedRectangle (rect, 3.0f);
    const Pt pivot (rect.getCentreX(), rect.getBottom() + 28.0f);
    const float radius = rect.getHeight() + 2.0f;
    const float sweep = 40.0f;

    auto map = [&] (float v)
    {
        float t;
        if (gr) t = 1.0f - std::pow (juce::jlimit (0.0f, 20.0f, v) / 20.0f, 0.6f);   // right = 0 dB GR
        else
        {
            // -20..+3, roughly logarithmic like a real VU
            const float lin = std::pow (10.0f, juce::jlimit (-20.0f, 3.0f, v) / 20.0f);
            t = (lin - 0.1f) / (std::pow (10.0f, 0.15f) - 0.1f);
        }
        return (-sweep + 2.0f * sweep * juce::jlimit (0.0f, 1.0f, t)) * pi / 180.0f;
    };
    struct Mark { float v; const char* t; };
    static const Mark grMarks[] = { { 0, "0" }, { 1, "" }, { 2, "2" }, { 3, "" }, { 5, "5" }, { 7, "" }, { 10, "10" }, { 15, "" }, { 20, "20" } };
    static const Mark vuMarks[] = { { -20, "20" }, { -10, "10" }, { -7, "7" }, { -5, "5" }, { -3, "3" }, { -2, "" }, { -1, "" }, { 0, "0" }, { 1, "" }, { 2, "" }, { 3, "+3" } };
    const float redFrom = gr ? 10.0f : 0.0f, redTo = gr ? 20.0f : 3.0f;
    polyline (g, 20, [&] (int k) { return polar (pivot, radius - 22.0f, map (redFrom + (redTo - redFrom) * (float) k / 20.0f)); }, 4.0f, accent);
    const float lo = gr ? 0.0f : -20.0f, hi = gr ? 20.0f : 3.0f;
    polyline (g, 40, [&] (int k) { return polar (pivot, radius - 26.0f, map (lo + (hi - lo) * (float) k / 40.0f)); }, 1.2f, INK);
    auto drawMarks = [&] (const Mark* m, int count)
    {
        for (int k = 0; k < count; ++k)
        {
            const float a = map (m[k].v);
            const bool lng = m[k].t[0] != 0;
            line (g, polar (pivot, radius - 26.0f, a), polar (pivot, radius - (lng ? 36.0f : 32.0f), a), 1.2f, INK);
            if (lng) say (g, m[k].t, polar (pivot, radius - 14.0f, a), 0, 0, 11.0f, INK);
        }
    };
    if (gr) drawMarks (grMarks, (int) std::size (grMarks));
    else    drawMarks (vuMarks, (int) std::size (vuMarks));
    say (g, it.label, { rect.getCentreX(), rect.getBottom() - 34.0f }, 0, 0, 10.0f, INK_SOFT);
    say (g, gr ? "dB" : "VU", { rect.getCentreX(), rect.getBottom() - 21.0f }, 0, 0, 10.0f, INK_SOFT);
    {
        juce::Graphics::ScopedSaveState s (g);
        g.reduceClipRegion (rect.toNearestInt());
        line (g, pivot, polar (pivot, radius - 8.0f, map (value)), 2.0f, Colour (0xff1a120c));
    }
    juce::Path strip;
    strip.addRoundedRectangle (rect.getX(), rect.getBottom() - 10.0f, rect.getWidth(), 10.0f, 3.0f, 3.0f, false, false, true, true);
    g.setColour (BAKELITE_HI);
    g.fillPath (strip);
}

void PequodPanel::drawLamp (juce::Graphics& g, const Item& it) const
{
    const float level = juce::isPositiveAndBelow (it.m, (int) shown.size()) ? shown[(size_t) it.m] : 0.0f;
    const Pt c (it.x, it.y);
    const Colour off (0xff5a2a18), on (0xffff8a2a);
    if (level > 0.05f)
    {
        circle (g, c, 16.0f, on.withAlpha ((juce::uint8) (40.0f * level)));
        circle (g, c, 12.0f, on.withAlpha ((juce::uint8) (70.0f * level)));
    }
    circle (g, c, 8.0f, BAKELITE);
    circle (g, c, 6.0f, lerp (off, on, level));
    circle (g, c - Pt (2.0f, 2.0f), 1.8f, juce::Colours::white.withAlpha ((juce::uint8) (60.0f + 120.0f * level)));
}

void PequodPanel::drawRotor (juce::Graphics& g, const Item& it) const
{
    const Pt c (it.x, it.y);
    const float r = it.r;
    const float hornPhase = juce::isPositiveAndBelow (it.horn, (int) shown.size()) ? shown[(size_t) it.horn] : 0.0f;
    const float drumPhase = juce::isPositiveAndBelow (it.drum, (int) shown.size()) ? shown[(size_t) it.drum] : 0.0f;
    const Rect rect (c.x - r - 12.0f, c.y - r - 12.0f, r * 2.0f + 24.0f, r * 2.0f + 24.0f);
    g.setColour (BAKELITE);
    g.fillRoundedRectangle (rect.expanded (5.0f), 6.0f);
    g.setColour (Colour (0xff2a201a));
    g.fillRoundedRectangle (rect, 4.0f);
    for (float x = rect.getX() + 6.0f; x < rect.getRight() - 3.0f; x += 7.0f)
        line (g, { x, rect.getY() + 3.0f }, { x, rect.getBottom() - 3.0f }, 1.0f, Colour (0xff3a2e26));
    const float da = drumPhase * 2.0f * pi;
    ring (g, c, r * 0.95f, 1.0f, Colour (0xff55473c));
    polyline (g, 24, [&] (int k) { return polar (c, r * 0.95f, da - 1.1f + 2.2f * (float) k / 24.0f); }, 6.0f, lerp (accent, CREAM, 0.25f));
    const float ha = hornPhase * 2.0f * pi;
    const auto tip = polar (c, r * 0.8f, ha), back = polar (c, r * 0.8f, ha + pi);
    juce::Path horn;
    horn.addTriangle (c, polar (tip, r * 0.22f, ha + pi / 2.0f), polar (tip, r * 0.22f, ha - pi / 2.0f));
    g.setColour (CREAM);
    g.fillPath (horn);
    g.setColour (BAKELITE);
    g.strokePath (horn, juce::PathStrokeType (1.0f));
    line (g, c, back, 4.0f, Colour (0xff9a8c76));
    circle (g, c, r * 0.12f, BAKELITE_HI);
    say (g, "ROTOR", { c.x, rect.getBottom() + 16.0f }, 0, 0, 10.0f, INK_SOFT);
}

void PequodPanel::drawRadio (juce::Graphics& g, const Item& it) const
{
    const int current = indexOf (it.p);
    for (int k = 0; k < it.labels.size(); ++k)
    {
        const Rect r (it.x, it.y + (float) k * it.rowH, it.w, it.rowH - 3.0f);
        const bool on = k == current;
        const bool hovered = r.contains (lastMouse) && isMouseOver();
        if (on || hovered)
        {
            g.setColour (on ? accent : lerp (PLATE, PLATE_EDGE, 0.35f));
            g.fillRoundedRectangle (r, 4.0f);
        }
        const Pt lc (r.getX() + 12.0f, r.getCentreY());
        ring (g, lc, 4.5f, 1.2f, on ? CREAM : INK_SOFT);
        if (on) circle (g, lc, 2.6f, CREAM);
        say (g, it.labels[k], lc + Pt (12.0f, 0.0f), -1, 0, 12.5f, on ? CREAM : INK);
    }
}

void PequodPanel::drawCaption (juce::Graphics& g, const Item& it) const
{
    const int k = juce::jmin (indexOf (it.p), it.labels.size() - 1);
    if (k < 0) return;
    juce::AttributedString s;
    s.append (it.labels[k], font (it.size), INK);
    s.setWordWrap (juce::AttributedString::byWord);
    juce::TextLayout layout;
    layout.createLayout (s, it.w);
    layout.draw (g, Rect (it.x, it.y, it.w, layout.getHeight() + 2.0f));
}

void PequodPanel::drawRoomPlan (juce::Graphics& g, const Item& it) const
{
    if (it.plans.empty()) return;
    const int k = juce::jmin (indexOf (it.room), (int) it.plans.size() - 1);
    const auto& plan = it.plans[(size_t) juce::jmax (0, k)];
    const float distance = plain (it.distance), spacing = plain (it.spacing);
    const Rect rect (it.x, it.y, it.w, it.h);
    g.setColour (Colour (0xffdcd0b3));
    g.fillRoundedRectangle (rect, 6.0f);
    const auto inner = rect.reduced (16.0f);
    const float scale = juce::jmin (inner.getWidth() / plan.length, inner.getHeight() / plan.width);
    const auto room = Rect (plan.length * scale, plan.width * scale).withCentre (inner.getCentre());
    auto to = [&] (float x, float y) { return Pt (room.getX() + x * scale, room.getY() + y * scale); };
    if (plan.openAir)
    {
        g.setColour (Colour (0xffc9bc9c));
        g.fillRect (room);
        // the building across the street, beyond the far end of the roof
        line (g, room.getTopRight(), room.getBottomRight(), 1.5f, INK_SOFT);
        for (int j = 0; j < 6; ++j)
        {
            const float y = room.getY() + room.getHeight() * ((float) j + 0.5f) / 6.0f;
            line (g, { room.getRight() + 4.0f, y - 4.0f }, { room.getRight() + 4.0f, y + 4.0f }, 3.0f, INK_SOFT);
        }
    }
    else
    {
        g.setColour (Colour (0xffe9dfc8));
        g.fillRect (room);
        g.setColour (INK);
        g.drawRect (room.expanded (3.0f), 3.0f);
    }
    if (plan.farWall.isNotEmpty())
        say (g, plan.farWall, { room.getRight() - 4.0f, room.getCentreY() }, 1, 0, 9.5f, INK_SOFT);
    // scale bar: 5 m
    const float bar = 5.0f * scale;
    const Pt b0 (rect.getX() + 10.0f, rect.getBottom() - 7.0f);
    line (g, b0, b0 + Pt (bar, 0.0f), 1.5f, INK_SOFT);
    say (g, "5 m", b0 + Pt (bar + 4.0f, 0.0f), -1, 0, 9.5f, INK_SOFT);
    say (g, juce::String (plan.length, 1) + " x " + juce::String (plan.width, 1) + " m", { rect.getRight() - 8.0f, rect.getBottom() - 7.0f }, 1, 0, 9.5f, INK_SOFT);

    const auto s = to (plan.srcX, plan.srcY);
    const float mx = juce::jmax (plan.srcX + 0.05f, juce::jmin (plan.srcX + distance, plan.length - 0.3f));
    const float half = 0.5f * spacing;
    std::vector<Pt> mics;
    if (half > 0.02f) mics = { to (mx, plan.srcY - half), to (mx, plan.srcY + half) };
    else              mics = { to (mx, plan.srcY) };
    for (auto m : mics) line (g, s, m, 1.0f, lerp (INK_SOFT, PLATE, 0.4f));
    circle (g, s, 7.0f, accent);
    say (g, "S", s, 0, 0, 9.0f, CREAM);
    for (auto m : mics)
    {
        circle (g, m, 5.5f, INK);
        circle (g, m, 2.0f, CREAM);
    }
    say (g, "band", s + Pt (0.0f, 14.0f), 0, 0, 9.5f, INK_SOFT);
    say (g, "mic " + juce::String (distance, 1) + " m", mics[0] + Pt (0.0f, -12.0f), 0, 0, 9.5f, INK);
}

void PequodPanel::drawPolar (juce::Graphics& g, const Item& it) const
{
    const Pt c (it.x, it.y);
    const float r = it.r;
    const float pattern = juce::isPositiveAndBelow (it.pattern, (int) shown.size()) ? shown[(size_t) it.pattern] : 2.0f;
    const int pat = juce::roundToInt (pattern);
    const float k = pat == 1 ? 0.0f : pat == 3 ? 1.0f : 0.5f;
    for (float fr : { 0.25f, 0.5f, 0.75f, 1.0f })
        ring (g, c, r * fr, fr == 1.0f ? 1.2f : 0.7f, PLATE_EDGE);
    line (g, c - Pt (r, 0.0f), c + Pt (r, 0.0f), 0.7f, PLATE_EDGE);
    line (g, c - Pt (0.0f, r), c + Pt (0.0f, r), 0.7f, PLATE_EDGE);
    // pattern: 0 deg (front) points up; the rear lobe (opposite polarity) drawn lighter
    juce::Path front, back;
    int frontN = 0, backN = 0;
    for (int n = 0; n <= 120; ++n)
    {
        const float th = (float) n / 120.0f * 2.0f * pi;
        const float gain = (1.0f - k) + k * std::cos (th);
        const Pt pt (c.x + r * std::abs (gain) * std::sin (th), c.y - r * std::abs (gain) * std::cos (th));
        auto& path = gain >= 0.0f ? front : back;
        auto& count = gain >= 0.0f ? frontN : backN;
        if (count++ == 0) path.startNewSubPath (pt); else path.lineTo (pt);
    }
    g.setColour (accent);
    g.strokePath (front, juce::PathStrokeType (2.2f));
    if (backN > 1)
    {
        g.setColour (lerp (accent, PLATE, 0.45f));
        g.strokePath (back, juce::PathStrokeType (1.6f));
    }
    const float angleDeg = plain (it.angle);
    const float a = angleDeg * pi / 180.0f;
    const Pt s (c.x + (r + 14.0f) * std::sin (a), c.y - (r + 14.0f) * std::cos (a));
    line (g, c, s, 1.0f, INK_SOFT);
    circle (g, s, 6.0f, INK);
    say (g, "source " + juce::String (juce::roundToInt (angleDeg)) + juce::String (juce::CharPointer_UTF8 ("\xc2\xb0")), s + Pt (10.0f, 0.0f), -1, 0, 9.5f, INK_SOFT);
    circle (g, c, 3.0f, INK);
    say (g, pat == 1 ? "OMNI" : pat == 3 ? "FIGURE-8" : "CARDIOID", c + Pt (0.0f, r + 12.0f), 0, 0, 10.0f, INK);
}

void PequodPanel::drawTuner (juce::Graphics& g, const Item& it) const
{
    const Rect rect (it.x, it.y, it.w, it.h);
    const float freq = juce::isPositiveAndBelow (it.freq, (int) shown.size()) ? shown[(size_t) it.freq] : 0.0f;
    const float a4 = it.reference >= 0 ? plain (it.reference) : 440.0f;
    g.setColour (BAKELITE);
    g.fillRoundedRectangle (rect.expanded (5.0f), 8.0f);
    g.setColour (Colour (0xff141210));
    g.fillRoundedRectangle (rect, 5.0f);
    const Colour glow (0xffffb24a), dimC (0xff4a3a2a), green (0xff7ce07a);

    const float scaleY = rect.getY() + 26.0f;
    auto xOf = [&] (float cents) { return rect.getCentreX() + juce::jlimit (-50.0f, 50.0f, cents) / 50.0f * (rect.getWidth() * 0.42f); };
    for (int c = -50; c <= 50; c += 10)
    {
        const float x = xOf ((float) c);
        const bool lng = c % 50 == 0 || c == 0;
        line (g, { x, scaleY }, { x, scaleY + (lng ? 12.0f : 7.0f) }, 1.2f, dimC);
    }
    say (g, "-50", { xOf (-50.0f), scaleY - 10.0f }, 0, 0, 10.0f, dimC);
    say (g, "+50", { xOf (50.0f), scaleY - 10.0f }, 0, 0, 10.0f, dimC);
    say (g, "0", { xOf (0.0f), scaleY - 10.0f }, 0, 0, 10.0f, dimC);

    if (freq > 0.0f && a4 > 0.0f)
    {
        const float n = 12.0f * std::log2 (freq / a4) + 69.0f;
        const float nearest = std::round (n);
        const float cents = (n - nearest) * 100.0f;
        const int idx = (((int) nearest % 12) + 12) % 12;
        const int oct = (int) nearest / 12 - 1;
        const bool inTune = std::abs (cents) < 3.0f;
        const auto col = inTune ? green : glow;
        const float x = xOf (cents);
        line (g, { x, scaleY - 4.0f }, { x, scaleY + 18.0f }, 3.0f, col);
        const float arrowY = rect.getCentreY() + 8.0f;
        const Pt left (rect.getX() + 34.0f, arrowY), right (rect.getRight() - 34.0f, arrowY);
        juce::Path l, rp;
        l.addTriangle (left + Pt (10.0f, -12.0f), left + Pt (-10.0f, 0.0f), left + Pt (10.0f, 12.0f));
        rp.addTriangle (right + Pt (-10.0f, -12.0f), right + Pt (10.0f, 0.0f), right + Pt (-10.0f, 12.0f));
        g.setColour (cents > 3.0f ? glow : dimC);
        g.fillPath (l);
        g.setColour (cents < -3.0f ? glow : dimC);
        g.fillPath (rp);
        say (g, noteNames[idx], { rect.getCentreX() - 6.0f, arrowY }, 0, 0, 54.0f, col);
        say (g, juce::String (oct), { rect.getCentreX() + 34.0f, arrowY + 14.0f }, -1, 0, 16.0f, col);
        say (g, juce::String (freq, 1) + " Hz   " + (cents >= 0 ? "+" : "") + juce::String (juce::roundToInt (cents)) + " cents",
              { rect.getCentreX(), rect.getBottom() - 16.0f }, 0, 0, 12.0f,
              Colour ((juce::uint8) juce::jmin (255, (int) (0x4a * 2.2f)), (juce::uint8) juce::jmin (255, (int) (0x3a * 2.2f)), (juce::uint8) juce::jmin (255, (int) (0x2a * 2.2f))));
        if (inTune) circle (g, { rect.getRight() - 16.0f, rect.getY() + 14.0f }, 5.0f, green);
    }
    else
        say (g, tunerLevel < 0.002f ? "PLAY A NOTE" : "LISTENING...", rect.getCentre(), 0, 0, 18.0f, dimC);
}

juce::Rectangle<float> PequodPanel::irButton (const Item& it) const { return { it.x, it.y, 84.0f, 24.0f }; }
juce::Rectangle<float> PequodPanel::irClear (const Item& it) const { return Rect (18.0f, 18.0f).withCentre ({ it.x + it.w - 10.0f, it.y + 12.0f }); }

const PequodPanel::Item* PequodPanel::irItem() const
{
    for (auto& it : items)
        if (it.kind == Kind::irloader) return &it;
    return nullptr;
}

void PequodPanel::drawIrLoader (juce::Graphics& g, const Item& it) const
{
    const auto btn = irButton (it);
    g.setColour (hoverButton == 4 ? lerp (accent, CREAM, 0.15f) : accent);
    g.fillRoundedRectangle (btn, 4.0f);
    say (g, "LOAD IR...", btn.getCentre(), 0, 0, 12.0f, CREAM);

    bool loaded = false;
    const auto status = inst.irStatus (loaded);
    const bool selected = it.cab >= 0 && indexOf (it.cab) == it.irPos;
    auto shownText = status.isEmpty() ? juce::String (selected ? "no IR loaded: using the built-in cabinet" : "no IR loaded") : status;
    const float maxW = it.w - 96.0f - 30.0f;
    const auto fnt = font (12.0f);
    while (shownText.length() > 4 && juce::GlyphArrangement::getStringWidth (fnt, shownText) > maxW)
        shownText = shownText.dropLastCharacters (2).trimCharactersAtEnd (juce::String (juce::CharPointer_UTF8 ("\xe2\x80\xa6"))) + juce::String (juce::CharPointer_UTF8 ("\xe2\x80\xa6"));
    say (g, shownText, { it.x + 96.0f, it.y + 12.0f }, -1, 0, 12.0f, loaded ? INK : INK_SOFT);
    if (loaded)
    {
        const auto xr = irClear (it);
        const auto c = xr.getCentre();
        ring (g, c, 8.0f, 1.2f, hoverButton == 5 ? INK : INK_SOFT);
        const float d = 3.5f;
        for (float s : { -1.0f, 1.0f })
            line (g, c + Pt (-d, -d * s), c + Pt (d, d * s), 1.4f, INK_SOFT);
    }
}

//==============================================================================
juce::Point<float> PequodPanel::presetCentre() const { return { (float) panelW / 2.0f, (float) panelH - 36.0f }; }
juce::Rectangle<float> PequodPanel::presetNameRect() const { return Rect (220.0f, 20.0f).withCentre (presetCentre()); }
juce::Rectangle<float> PequodPanel::presetPrevRect() const { return Rect (22.0f, 20.0f).withCentre (presetCentre() - Pt (128.0f, 0.0f)); }
juce::Rectangle<float> PequodPanel::presetNextRect() const { return Rect (22.0f, 20.0f).withCentre (presetCentre() + Pt (128.0f, 0.0f)); }

juce::Rectangle<float> PequodPanel::presetListRect() const
{
    const auto name = presetNameRect();
    const float listH = 22.0f * (float) presets.size() + 8.0f;
    return { name.getX(), name.getY() - listH - 4.0f, name.getWidth(), listH };
}

juce::Rectangle<float> PequodPanel::presetRowRect (int k) const
{
    const auto l = presetListRect();
    return { l.getX() + 4.0f, l.getY() + 4.0f + (float) k * 22.0f, l.getWidth() - 8.0f, 22.0f };
}

void PequodPanel::drawPresetBar (juce::Graphics& g) const
{
    const auto name = presetNameRect(), prev = presetPrevRect(), next = presetNextRect();
    say (g, "PRESET", { prev.getX() - 12.0f, prev.getCentreY() }, 1, 0, 11.0f, INK_SOFT);
    for (int k = 0; k < 2; ++k)
    {
        const auto r = k == 0 ? prev : next;
        g.setColour (hoverButton == k + 1 ? accent : BAKELITE);
        g.fillRoundedRectangle (r, 4.0f);
        const auto cx = r.getCentre();
        juce::Path tri;
        if (k == 1) tri.addTriangle (cx + Pt (-3.0f, -5.0f), cx + Pt (4.0f, 0.0f), cx + Pt (-3.0f, 5.0f));
        else        tri.addTriangle (cx + Pt (3.0f, -5.0f), cx + Pt (-4.0f, 0.0f), cx + Pt (3.0f, 5.0f));
        g.setColour (CREAM);
        g.fillPath (tri);
    }
    g.setColour (BAKELITE);
    g.fillRoundedRectangle (name, 4.0f);
    g.setColour (hoverButton == 3 || presetOpen ? accent : BAKELITE_HI);
    g.drawRoundedRectangle (name.reduced (0.5f), 4.0f, 1.0f);
    const auto label = juce::isPositiveAndBelow (currentPreset, (int) presets.size())
                           ? presets[(size_t) currentPreset].name + (presetDirty ? " *" : "")
                           : juce::String (juce::CharPointer_UTF8 ("\xe2\x80\x94 choose a preset \xe2\x80\x94"));
    say (g, label, name.getCentre(), 0, 0, 12.0f, METER_FACE);
}

void PequodPanel::drawPresetList (juce::Graphics& g) const
{
    const auto l = presetListRect();
    g.setColour (blackAlpha (90));
    g.fillRoundedRectangle (l.translated (0.0f, 3.0f), 6.0f);
    g.setColour (BAKELITE);
    g.fillRoundedRectangle (l, 6.0f);
    for (int k = 0; k < (int) presets.size(); ++k)
    {
        const auto r = presetRowRect (k);
        const bool hovered = presetHover == k;
        if (hovered || k == currentPreset)
        {
            g.setColour (hovered ? accent : BAKELITE_HI);
            g.fillRoundedRectangle (r, 4.0f);
        }
        say (g, presets[(size_t) k].name, { r.getX() + 10.0f, r.getCentreY() }, -1, 0, 12.0f, hovered ? CREAM : METER_FACE);
    }
}

void PequodPanel::applyPreset (int index)
{
    if (! juce::isPositiveAndBelow (index, (int) presets.size())) return;
    auto& ps = inst.getParameters();
    auto& pr = presets[(size_t) index];
    for (int p = 0; p < ps.size(); ++p)
    {
        // parameters the preset doesn't list go back to their defaults
        float norm = ps[p]->getDefaultValue();
        for (auto [k, v] : pr.values)
            if (k == p) norm = inst.normalise (p, v);
        ps[p]->beginChangeGesture();
        ps[p]->setValueNotifyingHost (norm);
        ps[p]->endChangeGesture();
    }
    currentPreset = index;
    presetDirty = false;
    presetOpen = false;
    repaint();
}

//==============================================================================
int PequodPanel::hitTest (Pt p) const
{
    for (int k = (int) items.size() - 1; k >= 0; --k)
    {
        auto& it = items[(size_t) k];
        const Pt c (it.x, it.y);
        switch (it.kind)
        {
            case Kind::knob:
                if (Rect (it.r * 2.0f + 16.0f, it.r * 2.0f + 16.0f).withCentre (c).contains (p)) return k;
                break;
            case Kind::sw:
                if (Rect (c.x - 62.0f, c.y - 11.0f, 124.0f, 22.0f).contains (p)) return k;
                break;
            case Kind::selector:
                if (Rect (58.0f, 58.0f).withCentre (c).contains (p)) return k;
                break;
            case Kind::radio:
                if (Rect (it.x, it.y, it.w, it.rowH * (float) it.labels.size()).contains (p)) return k;
                break;
            default: break;
        }
    }
    return -1;
}

void PequodPanel::mouseDown (const juce::MouseEvent& e)
{
    const auto p = e.position;
    dragged = false;
    dragItem = -1;

    if (presetOpen)
    {
        for (int k = 0; k < (int) presets.size(); ++k)
            if (presetRowRect (k).contains (p)) { applyPreset (k); return; }
        if (! presetNameRect().contains (p)) { presetOpen = false; repaint(); return; }
    }
    if (! presets.empty())
    {
        const int n = (int) presets.size();
        if (presetPrevRect().contains (p)) { applyPreset (currentPreset < 0 ? n - 1 : (currentPreset + n - 1) % n); return; }
        if (presetNextRect().contains (p)) { applyPreset (currentPreset < 0 ? 0 : (currentPreset + 1) % n); return; }
        if (presetNameRect().contains (p)) { presetOpen = ! presetOpen; repaint(); return; }
    }
    if (auto* ir = irItem())
    {
        if (irButton (*ir).contains (p))
        {
            chooser = std::make_unique<juce::FileChooser> ("Load cabinet impulse response", juce::File(), "*.wav;*.WAV");
            juce::Component::SafePointer<PequodPanel> safe (this);
            chooser->launchAsync (juce::FileBrowserComponent::openMode | juce::FileBrowserComponent::canSelectFiles,
                                  [safe] (const juce::FileChooser& fc)
                                  {
                                      if (safe == nullptr) return;
                                      const auto file = fc.getResult();
                                      if (file.existsAsFile())
                                      {
                                          safe->inst.loadIr (file);
                                          safe->inst.updateHostDisplay (juce::AudioProcessorListener::ChangeDetails().withNonParameterStateChanged (true));
                                      }
                                      safe->repaint();
                                  });
            return;
        }
        bool loaded = false;
        inst.irStatus (loaded);
        if (loaded && irClear (*ir).contains (p))
        {
            inst.unloadIr();
            inst.updateHostDisplay (juce::AudioProcessorListener::ChangeDetails().withNonParameterStateChanged (true));
            repaint();
            return;
        }
    }

    dragItem = hitTest (p);
    if (dragItem < 0) return;
    auto& it = items[(size_t) dragItem];
    dragStart = lastDrag = p;
    dragStartValue = value (it.p);
    selectorAcc = 0;

    switch (it.kind)
    {
        case Kind::knob:
            if (auto* o = paramObj (it.p)) o->beginChangeGesture();
            break;
        case Kind::sw:
            setValue (it.p, value (it.p) > 0.5f ? 0.0f : 1.0f, true);
            dragItem = -1;
            break;
        case Kind::radio:
        {
            const int k = (int) ((p.y - it.y) / it.rowH);
            if (k != indexOf (it.p)) setIndex (it.p, k);
            dragItem = -1;
            break;
        }
        default: break;
    }
}

void PequodPanel::mouseDrag (const juce::MouseEvent& e)
{
    if (dragItem < 0) return;
    auto& it = items[(size_t) dragItem];
    if (e.getDistanceFromDragStart() > 2) dragged = true;
    if (it.kind == Kind::knob)
    {
        const float sens = e.mods.isShiftDown() ? 0.0012f : 0.005f;
        if (auto* o = paramObj (it.p))
        {
            // relative, so Shift can be pressed mid-drag
            const float v = juce::jlimit (0.0f, 1.0f, o->getValue() - (e.position.y - lastDrag.y) * sens);
            o->setValueNotifyingHost (v);
            edited();
        }
    }
    else if (it.kind == Kind::selector && dragged)
    {
        selectorAcc += (e.position.x - lastDrag.x) - (e.position.y - lastDrag.y);
        if (std::abs (selectorAcc) > 18.0f)
        {
            const int cur = indexOf (it.p);
            const int k = selectorAcc > 0 ? juce::jmin (cur + 1, stepsOf (it.p)) : juce::jmax (cur - 1, 0);
            if (k != cur) setIndex (it.p, k);
            selectorAcc = 0;
        }
    }
    lastDrag = e.position;
}

void PequodPanel::mouseUp (const juce::MouseEvent&)
{
    if (dragItem >= 0)
    {
        auto& it = items[(size_t) dragItem];
        if (it.kind == Kind::knob)
        {
            if (auto* o = paramObj (it.p)) o->endChangeGesture();
        }
        else if (it.kind == Kind::selector && ! dragged)
            setIndex (it.p, (indexOf (it.p) + 1) % (stepsOf (it.p) + 1));   // a click steps on
    }
    dragItem = -1;
    repaint();
}

void PequodPanel::mouseDoubleClick (const juce::MouseEvent& e)
{
    const int k = hitTest (e.position);
    if (k < 0 || items[(size_t) k].kind != Kind::knob) return;
    if (auto* o = paramObj (items[(size_t) k].p))
        setValue (items[(size_t) k].p, o->getDefaultValue(), true);
}

void PequodPanel::mouseWheelMove (const juce::MouseEvent& e, const juce::MouseWheelDetails& w)
{
    const int k = hitTest (e.position);
    if (k < 0 || w.deltaY == 0.0f) return;
    auto& it = items[(size_t) k];
    if (it.kind == Kind::knob)
        setValue (it.p, value (it.p) + w.deltaY * (e.mods.isShiftDown() ? 0.12f : 0.4f), true);
    else if (it.kind == Kind::selector)
    {
        const int cur = indexOf (it.p);
        const int n = w.deltaY > 0 ? juce::jmin (cur + 1, stepsOf (it.p)) : juce::jmax (cur - 1, 0);
        if (n != cur) setIndex (it.p, n);
    }
}

void PequodPanel::mouseMove (const juce::MouseEvent& e)
{
    lastMouse = e.position;
    const int k = hitTest (e.position);
    int button = 0;
    if (! presets.empty())
    {
        if (presetPrevRect().contains (lastMouse)) button = 1;
        else if (presetNextRect().contains (lastMouse)) button = 2;
        else if (presetNameRect().contains (lastMouse)) button = 3;
    }
    if (auto* ir = irItem())
    {
        if (irButton (*ir).contains (lastMouse)) button = 4;
        else if (irClear (*ir).contains (lastMouse)) button = 5;
    }
    int ph = -1;
    if (presetOpen)
        for (int j = 0; j < (int) presets.size(); ++j)
            if (presetRowRect (j).contains (lastMouse)) ph = j;
    if (k != hoverItem || button != hoverButton || ph != presetHover || (k >= 0 && items[(size_t) k].kind == Kind::radio))
    {
        hoverItem = k;
        hoverButton = button;
        presetHover = ph;
        repaint();
    }
    const auto kind = k >= 0 ? items[(size_t) k].kind : Kind::unknown;
    setMouseCursor (kind == Kind::knob ? juce::MouseCursor::UpDownResizeCursor
                    : (k >= 0 || button != 0 || ph >= 0) ? juce::MouseCursor::PointingHandCursor
                    : juce::MouseCursor::NormalCursor);
}

void PequodPanel::mouseExit (const juce::MouseEvent&)
{
    hoverItem = -1;
    hoverButton = 0;
    presetHover = -1;
    lastMouse = { -100.0f, -100.0f };
    repaint();
}

juce::String PequodPanel::getTooltip()
{
    switch (hoverButton)
    {
        case 1: return "Previous preset";
        case 2: return "Next preset";
        case 3: return "Click for the preset list";
        case 4: return "Load a cabinet impulse response (WAV, any sample rate; the first 250 ms is used). Select IR on the CAB switch to hear it.";
        case 5: return "Unload the IR";
        default: break;
    }
    const int k = hitTest (lastMouse);
    if (k < 0) return {};
    auto& it = items[(size_t) k];
    switch (it.kind)
    {
        case Kind::knob:     return it.tip + "\n\nDrag up/down (Shift = fine), scroll, double-click to reset.";
        case Kind::sw:       return it.tip + "\n\nClick to switch.";
        case Kind::selector: return it.tip + "\n\nClick to step, drag or scroll to choose.";
        default:             return it.tip;
    }
}

//==============================================================================
void PequodPanel::timerCallback()
{
    const double now = juce::Time::getMillisecondCounterHiRes() * 0.001;
    const float dt = juce::jlimit (0.001f, 0.1f, (float) (now - lastTick));
    lastTick = now;
    bool dirty = false;

    // meters: read + reset the max-hold values, then the VST3 panel's ballistics
    std::fill (raw.begin(), raw.end(), 0.0f);
    inst.takeMeters (raw.data(), (int) raw.size());
    auto at = [this] (int m) -> float& { static float dummy; return juce::isPositiveAndBelow (m, (int) shown.size()) ? shown[(size_t) m] : dummy; };
    auto in = [this] (int m) { return juce::isPositiveAndBelow (m, (int) raw.size()) ? raw[(size_t) m] : 0.0f; };
    for (auto& it : items)
    {
        switch (it.kind)
        {
            case Kind::meter:
            {
                const bool gr = it.meterKind == "gr";
                const float target = gr ? in (it.m) : vuDb (in (it.m));
                float& s = at (it.m);
                const float tau = gr ? (target > s ? 0.03f : 0.35f) : 0.3f;
                s += (target - s) * (1.0f - std::exp (-dt / tau));
                dirty = true;
                break;
            }
            case Kind::lamp:
            {
                const float target = juce::jlimit (0.0f, 1.0f, (in (it.m) - it.lo) / (it.hi - it.lo));
                float& s = at (it.m);
                const float tau = target > s ? 0.02f : 0.25f;
                const float before = s;
                s += (target - s) * (1.0f - std::exp (-dt / tau));
                dirty = dirty || std::abs (s - before) > 1.0e-4f;
                break;
            }
            case Kind::tuner:
            {
                const float fr = in (it.freq);
                tunerLevel = juce::jmax (in (it.level), tunerLevel * 0.9f);
                float& cur = at (it.freq);
                if (fr > 0.0f)
                {
                    // jump straight to a new note, glide within the same one
                    if (cur <= 0.0f || std::abs (fr / cur - 1.0f) > 0.03f) cur = fr;
                    else cur = cur * std::pow (fr / cur, 1.0f - std::exp (-dt / 0.08f));
                    tunerSeen = now;
                }
                else if (now - tunerSeen > 1.5)
                    cur = 0.0f;
                dirty = true;
                break;
            }
            case Kind::polar:
                if (in (it.pattern) > 0.0f && at (it.pattern) != in (it.pattern)) { at (it.pattern) = in (it.pattern); dirty = true; }
                break;
            case Kind::rotor:
                for (int m : { it.horn, it.drum })
                    if (in (m) > 0.0f) { at (m) = in (m); dirty = true; }
                break;
            case Kind::irloader:
                dirty = true;   // status text after a load
                break;
            default: break;
        }
    }

    // host-side changes (automation, presets loaded by the host)
    auto& ps = inst.getParameters();
    for (int p = 0; p < ps.size() && p < (int) lastValues.size(); ++p)
    {
        const float v = ps[p]->getValue();
        if (v != lastValues[(size_t) p]) { lastValues[(size_t) p] = v; dirty = true; }
    }
    if (dirty) repaint();
}
