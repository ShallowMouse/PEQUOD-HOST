#include "PequodPanel.h"
#include "PequodLookAndFeel.h"

namespace
{
    const juce::Colour plate   { 0xffe8ddc4 };
    const juce::Colour plateLo { 0xffd6c9ad };
    const juce::Colour ink     { 0xff2b2118 };
    const juce::Colour inkDim  { 0xff6b5a48 };
    const juce::Colour wood    { 0xff3a2416 };
    const juce::Colour knobFace{ 0xff1c1714 };
    constexpr float footerH = 34.0f;
    constexpr float knobStart = juce::MathConstants<float>::pi * 1.25f;   // -135 deg from 12 o'clock
    constexpr float knobSweep = juce::MathConstants<float>::pi * 1.5f;

    float f (const juce::var& v, float d = 0) { return v.isVoid() ? d : (float) (double) v; }
    int i (const juce::var& v, int d = -1) { return v.isVoid() ? d : (int) v; }

    juce::StringArray strings (const juce::var& v)
    {
        juce::StringArray s;
        if (auto* a = v.getArray())
            for (auto& x : *a) s.add (x.toString());
        return s;
    }

    juce::Font ui (float size, bool bold = false) { return Pequod::body (size, bold); }
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
    accent = juce::Colour::fromString ("ff" + j["accent"].toString().removeCharacters ("#"));
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
            items.push_back (it);
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

    meters.assign ((size_t) juce::jmax (1, inst.info().meters), 0.0f);
    shownMeters = meters;
    lastValues.assign (inst.getParameters().size(), -1.0f);

    setSize (panelW, panelH);
    setWantsKeyboardFocus (false);
    startTimerHz (30);
}

PequodPanel::~PequodPanel()
{
    stopTimer();
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

int PequodPanel::stepsOf (int p) const
{
    auto& ps = inst.info().params;
    return juce::isPositiveAndBelow (p, (int) ps.size()) ? ps[(size_t) p].steps : 0;
}

int PequodPanel::indexOf (int p) const
{
    const int s = stepsOf (p);
    return s > 0 ? juce::roundToInt (value (p) * (float) s) : juce::roundToInt (value (p));
}

void PequodPanel::setValue (int p, float v)
{
    if (auto* o = paramObj (p))
    {
        v = juce::jlimit (0.0f, 1.0f, v);
        if (std::abs (o->getValue() - v) > 1.0e-6f)
            o->setValueNotifyingHost (v);
        repaint();
    }
}

void PequodPanel::setIndex (int p, int index)
{
    const int s = stepsOf (p);
    if (s > 0) setValue (p, (float) juce::jlimit (0, s, index) / (float) s);
}

bool PequodPanel::isActive (const Item& it) const
{
    if (it.activeP < 0) return true;
    const float idx = (float) indexOf (it.activeP);
    return idx >= it.activeMin - 0.01f && idx <= it.activeMax + 0.01f;
}

juce::String PequodPanel::text (int p) const
{
    if (auto* o = paramObj (p)) return o->getText (o->getValue(), 32);
    return {};
}

//==============================================================================
void PequodPanel::paint (juce::Graphics& g)
{
    auto b = getLocalBounds().toFloat();
    g.fillAll (wood);
    auto face = b.reduced (14.0f, 12.0f);
    g.setColour (plate);
    g.fillRoundedRectangle (face, 6.0f);
    g.setColour (plateLo);
    g.drawRoundedRectangle (face, 6.0f, 1.0f);

    // header
    auto head = face.withHeight (56.0f);
    {
        juce::Path p;
        p.addRoundedRectangle (head.getX(), head.getY(), head.getWidth(), head.getHeight(), 6.0f, 6.0f, true, true, false, false);
        g.setColour (accent);
        g.fillPath (p);
    }
    g.setColour (juce::Colour (0xfff6eedc));
    g.setFont (ui (30.0f));
    g.drawText (title, head.reduced (30.0f, 0).toNearestInt(), juce::Justification::centredLeft, false);
    g.setFont (ui (11.5f));
    auto subs = head.reduced (30.0f, 10.0f);
    g.drawText (sub1, subs.removeFromTop (subs.getHeight() * 0.5f).toNearestInt(), juce::Justification::bottomRight, false);
    g.drawText (sub2, subs.toNearestInt(), juce::Justification::topRight, false);

    for (auto& it : items)
    {
        switch (it.kind)
        {
            case Kind::knob:     drawKnob (g, it); break;
            case Kind::sw:       drawSwitch (g, it); break;
            case Kind::selector: drawSelector (g, it); break;
            case Kind::meter:    drawMeter (g, it); break;
            case Kind::lamp:     drawLamp (g, it); break;
            case Kind::radio:    drawRadio (g, it); break;
            case Kind::caption:  drawCaption (g, it); break;
            case Kind::roomplan: drawRoomPlan (g, it); break;
            case Kind::polar:    drawPolar (g, it); break;
            case Kind::tuner:    drawTuner (g, it); break;
            case Kind::rotor:    drawRotor (g, it); break;
            case Kind::divider:
                g.setColour (plateLo.darker (0.15f));
                g.drawHorizontalLine ((int) it.y, 50.0f, (float) panelW - 50.0f);
                break;
            case Kind::text:
                g.setColour (ink);
                g.setFont (ui (it.size));
                g.drawText (it.label, juce::Rectangle<float> (it.x - 200.0f, it.y - it.size, 400.0f, it.size * 2.0f).toNearestInt(),
                            juce::Justification::centred, false);
                break;
            case Kind::irloader:
                g.setColour (inkDim);
                g.setFont (ui (11.0f));
                g.drawText ("Impulse responses: use a PEQUOD Cabs plugin after the amp, or the VST3 version",
                            juce::Rectangle<float> (it.x, it.y - 10.0f, juce::jmax (200.0f, it.w), 20.0f).toNearestInt(),
                            juce::Justification::centredLeft, true);
                break;
            case Kind::unknown: break;
        }
    }

    // footer: mark and presets
    const float fy = (float) panelH - footerH;
    g.setColour (juce::Colour (0xffb08a3a));
    g.setFont (Pequod::heading (13.0f));
    g.drawText ("[PEQUOD]", juce::Rectangle<float> (34.0f, fy - 14.0f, 90.0f, 22.0f).toNearestInt(), juce::Justification::centredLeft, false);
    g.setColour (inkDim);
    g.setFont (ui (11.0f));
    g.drawText ("PRESET", juce::Rectangle<float> (130.0f, fy - 14.0f, 50.0f, 22.0f).toNearestInt(), juce::Justification::centredLeft, false);
    for (auto* r : { &prevBox, &nextBox })
    {
        g.setColour (knobFace);
        g.fillRoundedRectangle (*r, 3.0f);
    }
    g.setColour (plate);
    {
        juce::Path tri;
        auto c = prevBox.getCentre();
        tri.addTriangle (c.x + 4, c.y - 5, c.x + 4, c.y + 5, c.x - 4, c.y);
        auto d = nextBox.getCentre();
        tri.addTriangle (d.x - 4, d.y - 5, d.x - 4, d.y + 5, d.x + 4, d.y);
        g.fillPath (tri);
    }
    g.setColour (knobFace);
    g.fillRoundedRectangle (presetBox, 3.0f);
    g.setColour (juce::Colour (0xffe0c27a));
    g.setFont (ui (12.0f));
    g.drawText (juce::isPositiveAndBelow (currentPreset, (int) presets.size()) ? presets[(size_t) currentPreset].name : juce::String ("- choose a preset -"),
                presetBox.reduced (6.0f, 0).toNearestInt(), juce::Justification::centred, true);
}

void PequodPanel::resized()
{
    const float fy = (float) panelH - footerH;
    prevBox = { 190.0f, fy - 13.0f, 22.0f, 20.0f };
    presetBox = { 220.0f, fy - 13.0f, 220.0f, 20.0f };
    nextBox = { 448.0f, fy - 13.0f, 22.0f, 20.0f };
}

//==============================================================================
void PequodPanel::drawKnob (juce::Graphics& g, const Item& it) const
{
    const bool active = isActive (it);
    const float v = value (it.p);
    const auto c = juce::Point<float> (it.x, it.y);
    const float r = it.r;
    g.saveState();
    if (! active) g.setOpacity (0.4f);

    // ticks
    g.setColour (ink);
    for (int k = 0; k <= 10; ++k)
    {
        const float a = -knobStart + knobSweep * (float) k / 10.0f;
        auto p1 = c.getPointOnCircumference (r + 4.0f, a);
        auto p2 = c.getPointOnCircumference (r + (k % 5 == 0 ? 9.0f : 7.0f), a);
        g.drawLine ({ p1, p2 }, 1.2f);
    }
    // value arc
    juce::Path arc;
    arc.addCentredArc (c.x, c.y, r + 2.0f, r + 2.0f, 0.0f, -knobStart, -knobStart + knobSweep * v, true);
    g.setColour (accent);
    g.strokePath (arc, juce::PathStrokeType (3.0f));
    // body
    g.setColour (juce::Colours::black.withAlpha (0.25f));
    g.fillEllipse (c.x - r + 2.0f, c.y - r + 3.0f, r * 2.0f, r * 2.0f);
    g.setColour (knobFace);
    g.fillEllipse (c.x - r, c.y - r, r * 2.0f, r * 2.0f);
    g.setColour (juce::Colour (0xff3a332e));
    g.drawEllipse (c.x - r * 0.82f, c.y - r * 0.82f, r * 1.64f, r * 1.64f, 1.0f);
    // pointer
    const float a = -knobStart + knobSweep * v;
    g.setColour (juce::Colour (0xfff6eedc));
    g.drawLine ({ c.getPointOnCircumference (r * 0.25f, a), c.getPointOnCircumference (r * 0.82f, a) }, juce::jmax (2.0f, r * 0.09f));

    // label and value
    const float ls = juce::jlimit (10.0f, 16.0f, r * 0.42f);
    g.setColour (ink);
    g.setFont (ui (ls));
    g.drawText (it.label, juce::Rectangle<float> (c.x - 70.0f, c.y - r - 14.0f - ls, 140.0f, ls + 4.0f).toNearestInt(), juce::Justification::centred, false);
    g.setColour (inkDim);
    g.setFont (ui (juce::jlimit (10.0f, 13.0f, r * 0.36f)));
    g.drawText (text (it.p), juce::Rectangle<float> (c.x - 60.0f, c.y + r + 9.0f, 120.0f, 16.0f).toNearestInt(), juce::Justification::centred, false);
    g.restoreState();
}

void PequodPanel::drawSwitch (juce::Graphics& g, const Item& it) const
{
    const bool on = value (it.p) > 0.5f;
    auto body = juce::Rectangle<float> (it.x, it.y - 8.0f, 34.0f, 16.0f);
    g.setColour (on ? accent : knobFace);
    g.fillRoundedRectangle (body, 8.0f);
    g.setColour (plate);
    const float kx = on ? body.getRight() - 14.0f : body.getX() + 2.0f;
    g.fillEllipse (kx, body.getY() + 2.0f, 12.0f, 12.0f);
    g.setFont (ui (11.0f));
    if (it.labels.size() >= 2)
    {
        g.setColour (on ? inkDim : ink);
        g.drawText (it.labels[0], juce::Rectangle<float> (it.x - 64.0f, it.y - 9.0f, 58.0f, 18.0f).toNearestInt(), juce::Justification::centredRight, false);
        g.setColour (on ? ink : inkDim);
        g.drawText (it.labels[1], juce::Rectangle<float> (it.x + 40.0f, it.y - 9.0f, 80.0f, 18.0f).toNearestInt(), juce::Justification::centredLeft, false);
    }
}

juce::Point<float> PequodPanel::selectorLabelPos (const Item& it, int k) const
{
    const int n = juce::jmax (2, it.labels.size());
    const float spread = juce::jmin (knobSweep, 0.55f * (float) (n - 1));
    const float a = -spread * 0.5f + spread * (float) k / (float) (n - 1);
    return juce::Point<float> (it.x, it.y).getPointOnCircumference (34.0f, a);
}

void PequodPanel::drawSelector (juce::Graphics& g, const Item& it) const
{
    const int n = juce::jmax (2, it.labels.size());
    const int idx = juce::jlimit (0, n - 1, indexOf (it.p));
    const auto c = juce::Point<float> (it.x, it.y);
    const float r = 15.0f;
    g.setFont (ui (10.5f));
    for (int k = 0; k < it.labels.size(); ++k)
    {
        auto p = selectorLabelPos (it, k);
        g.setColour (k == idx ? accent.brighter (0.2f) : inkDim);
        g.drawText (it.labels[k], juce::Rectangle<float> (p.x - 30.0f, p.y - 7.0f, 60.0f, 14.0f).toNearestInt(), juce::Justification::centred, false);
        g.setColour (ink);
        g.drawLine ({ c.getPointOnCircumference (r + 2.0f, c.getAngleToPoint (selectorLabelPos (it, k)) ), c.getPointOnCircumference (r + 6.0f, c.getAngleToPoint (selectorLabelPos (it, k))) }, 1.0f);
    }
    g.setColour (knobFace);
    g.fillEllipse (c.x - r, c.y - r, r * 2.0f, r * 2.0f);
    const float a = c.getAngleToPoint (selectorLabelPos (it, idx));
    juce::Path pointer;
    pointer.addTriangle (c.getPointOnCircumference (r + 4.0f, a), c.getPointOnCircumference (r * 0.5f, a + 1.2f), c.getPointOnCircumference (r * 0.5f, a - 1.2f));
    g.setColour (juce::Colour (0xfff6eedc));
    g.fillPath (pointer);
    g.setColour (ink);
    g.setFont (ui (11.0f));
    g.drawText (it.label, juce::Rectangle<float> (c.x - 70.0f, c.y + r + 8.0f, 140.0f, 14.0f).toNearestInt(), juce::Justification::centred, false);
}

void PequodPanel::drawMeter (juce::Graphics& g, const Item& it) const
{
    auto box = juce::Rectangle<float> (it.x, it.y, it.w, it.h);
    g.setColour (knobFace);
    g.fillRoundedRectangle (box, 4.0f);
    auto face = box.reduced (6.0f);
    face.removeFromBottom (8.0f);
    g.setColour (juce::Colour (0xfff2e0a8));
    g.fillRect (face);
    const float v = juce::isPositiveAndBelow (it.m, (int) shownMeters.size()) ? shownMeters[(size_t) it.m] : 0.0f;
    // VU: linear peak, 0 VU = -18 dBFS, scale -20..+3. GR: dB 0..20 (needle swings left).
    float pos;
    if (it.meterKind == "gr")
        pos = 1.0f - juce::jlimit (0.0f, 20.0f, v) / 20.0f;
    else
    {
        const float vu = juce::Decibels::gainToDecibels (v, -100.0f) + 18.0f;
        pos = juce::jlimit (0.0f, 1.0f, (vu + 20.0f) / 23.0f);
    }
    const auto pivot = juce::Point<float> (face.getCentreX(), face.getBottom() + 4.0f);
    const float rad = face.getHeight() * 0.95f;
    juce::Path scale;
    scale.addCentredArc (pivot.x, pivot.y, rad * 0.85f, rad * 0.85f, 0.0f, -0.8f, 0.8f, true);
    g.setColour (ink);
    g.strokePath (scale, juce::PathStrokeType (1.0f));
    const float a = -0.8f + 1.6f * pos;
    g.drawLine ({ pivot, pivot.getPointOnCircumference (rad * 0.95f, a) }, 1.5f);
    g.setColour (inkDim);
    g.setFont (ui (10.0f));
    g.drawText (it.label, face.removeFromBottom (16.0f).toNearestInt(), juce::Justification::centred, false);
}

void PequodPanel::drawLamp (juce::Graphics& g, const Item& it) const
{
    const float v = juce::isPositiveAndBelow (it.m, (int) shownMeters.size()) ? shownMeters[(size_t) it.m] : 0.0f;
    const float k = juce::jlimit (0.0f, 1.0f, (v - it.lo) / juce::jmax (1.0e-6f, it.hi - it.lo));
    const auto c = juce::Point<float> (it.x, it.y);
    if (k > 0.02f)
    {
        g.setColour (juce::Colour (0xffffb040).withAlpha (0.35f * k));
        g.fillEllipse (c.x - 13.0f, c.y - 13.0f, 26.0f, 26.0f);
    }
    g.setColour (juce::Colour (0xff4a1c10).interpolatedWith (juce::Colour (0xffff8a2a), k));
    g.fillEllipse (c.x - 7.0f, c.y - 7.0f, 14.0f, 14.0f);
    g.setColour (ink);
    g.drawEllipse (c.x - 7.0f, c.y - 7.0f, 14.0f, 14.0f, 1.0f);
    g.setFont (ui (11.0f));
    g.drawText (it.label, juce::Rectangle<float> (c.x + 12.0f, c.y - 8.0f, 120.0f, 16.0f).toNearestInt(), juce::Justification::centredLeft, false);
}

void PequodPanel::drawRadio (juce::Graphics& g, const Item& it) const
{
    const int sel = indexOf (it.p);
    g.setFont (ui (12.5f));
    for (int k = 0; k < it.labels.size(); ++k)
    {
        auto row = juce::Rectangle<float> (it.x, it.y + it.rowH * (float) k, juce::jmax (120.0f, it.w), it.rowH - 2.0f);
        if (k == sel)
        {
            g.setColour (accent);
            g.fillRoundedRectangle (row, 3.0f);
        }
        g.setColour (k == sel ? plate : ink);
        g.drawEllipse (row.getX() + 6.0f, row.getCentreY() - 5.0f, 10.0f, 10.0f, 1.2f);
        if (k == sel) g.fillEllipse (row.getX() + 8.5f, row.getCentreY() - 2.5f, 5.0f, 5.0f);
        g.drawText (it.labels[k], row.withTrimmedLeft (24.0f).toNearestInt(), juce::Justification::centredLeft, true);
    }
}

void PequodPanel::drawCaption (juce::Graphics& g, const Item& it) const
{
    const int k = indexOf (it.p);
    if (! juce::isPositiveAndBelow (k, it.labels.size())) return;
    g.setColour (ink);
    g.setFont (ui (it.size));
    g.drawFittedText (it.labels[k], juce::Rectangle<float> (it.x, it.y, it.w, it.size * 4.0f).toNearestInt(), juce::Justification::topLeft, 3, 1.0f);
}

void PequodPanel::drawRoomPlan (juce::Graphics& g, const Item& it) const
{
    auto box = juce::Rectangle<float> (it.x, it.y, it.w, it.h);
    g.setColour (plateLo);
    g.fillRoundedRectangle (box, 4.0f);
    auto room = box.reduced (box.getWidth() * 0.28f, box.getHeight() * 0.12f);
    g.setColour (ink);
    g.drawRect (room, 2.0f);
    const float dist = it.distance >= 0 ? value (it.distance) : 0.2f;
    const auto src = juce::Point<float> (room.getX() + room.getWidth() * 0.25f, room.getCentreY());
    const auto mic = src.translated (juce::jlimit (6.0f, room.getWidth() * 0.7f, room.getWidth() * (0.1f + 0.7f * dist)), 0.0f);
    g.setColour (accent);
    g.fillEllipse (src.x - 6, src.y - 6, 12, 12);
    g.setColour (ink);
    g.drawLine ({ src, mic }, 1.0f);
    g.fillEllipse (mic.x - 4, mic.y - 4, 8, 8);
}

void PequodPanel::drawPolar (juce::Graphics& g, const Item& it) const
{
    const auto c = juce::Point<float> (it.x, it.y);
    const float pat = it.pattern >= 0 ? value (it.pattern) : 0.5f;   // omni .. figure-8
    juce::Path p;
    for (int k = 0; k <= 72; ++k)
    {
        const float a = juce::MathConstants<float>::twoPi * (float) k / 72.0f;
        const float gain = std::abs ((1.0f - pat) + pat * std::cos (a));
        auto q = c.getPointOnCircumference (it.r * gain, a);
        if (k == 0) p.startNewSubPath (q); else p.lineTo (q);
    }
    g.setColour (ink);
    g.drawEllipse (c.x - it.r, c.y - it.r, it.r * 2, it.r * 2, 0.6f);
    g.setColour (accent);
    g.strokePath (p, juce::PathStrokeType (1.5f));
}

void PequodPanel::drawTuner (juce::Graphics& g, const Item& it) const
{
    auto box = juce::Rectangle<float> (it.x, it.y, it.w, it.h);
    g.setColour (knobFace);
    g.fillRoundedRectangle (box, 4.0f);
    const float hz = juce::isPositiveAndBelow (it.freq, (int) shownMeters.size()) ? shownMeters[(size_t) it.freq] : 0.0f;
    const float ref = juce::isPositiveAndBelow (it.reference, (int) shownMeters.size()) && shownMeters[(size_t) it.reference] > 0 ? shownMeters[(size_t) it.reference] : 440.0f;
    g.setColour (plate);
    g.setFont (ui (28.0f, true));
    if (hz > 20.0f)
    {
        static const char* names[] = { "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B" };
        const float midi = 69.0f + 12.0f * std::log2 (hz / ref);
        const int n = juce::roundToInt (midi);
        const float cents = (midi - (float) n) * 100.0f;
        g.drawText (juce::String (names[((n % 12) + 12) % 12]) + juce::String (n / 12 - 1), box.removeFromTop (box.getHeight() * 0.6f).toNearestInt(), juce::Justification::centred, false);
        g.setFont (ui (13.0f));
        g.setColour (std::abs (cents) < 3.0f ? juce::Colour (0xff6fcf6f) : juce::Colour (0xffe0a82e));
        g.drawText ((cents >= 0 ? "+" : "") + juce::String (cents, 1) + " cents", box.toNearestInt(), juce::Justification::centred, false);
    }
    else
        g.drawText ("-", box.toNearestInt(), juce::Justification::centred, false);
}

void PequodPanel::drawRotor (juce::Graphics& g, const Item& it) const
{
    const auto c = juce::Point<float> (it.x, it.y);
    g.setColour (knobFace);
    g.fillEllipse (c.x - it.r, c.y - it.r, it.r * 2, it.r * 2);
    for (int k = 0; k < 2; ++k)
    {
        const float rr = it.r * (k == 0 ? 0.85f : 0.5f);
        g.setColour (k == 0 ? accent : plate);
        g.drawLine ({ c.getPointOnCircumference (-rr, rotorPhase[k]), c.getPointOnCircumference (rr, rotorPhase[k]) }, 3.0f);
    }
}

//==============================================================================
int PequodPanel::hitTest (juce::Point<float> p) const
{
    for (int k = (int) items.size() - 1; k >= 0; --k)
    {
        auto& it = items[(size_t) k];
        switch (it.kind)
        {
            case Kind::knob:
                if (p.getDistanceFrom ({ it.x, it.y }) <= it.r + 6.0f) return k;
                break;
            case Kind::sw:
                if (juce::Rectangle<float> (it.x - 64.0f, it.y - 12.0f, 160.0f, 24.0f).contains (p)) return k;
                break;
            case Kind::selector:
                if (p.getDistanceFrom ({ it.x, it.y }) <= 46.0f) return k;
                break;
            case Kind::radio:
                if (juce::Rectangle<float> (it.x, it.y, juce::jmax (120.0f, it.w), it.rowH * (float) it.labels.size()).contains (p)) return k;
                break;
            default: break;
        }
    }
    return -1;
}

void PequodPanel::mouseDown (const juce::MouseEvent& e)
{
    const auto p = e.position;
    if (prevBox.contains (p) || nextBox.contains (p))
    {
        if (presets.empty()) return;
        const int n = (int) presets.size();
        const int step = prevBox.contains (p) ? -1 : 1;
        applyPreset (currentPreset < 0 ? (step > 0 ? 0 : n - 1) : (currentPreset + step + n) % n);
        return;
    }
    if (presetBox.contains (p)) { showPresetMenu(); return; }

    dragItem = hitTest (p);
    if (dragItem < 0) return;
    auto& it = items[(size_t) dragItem];
    if (! isActive (it)) { dragItem = -1; return; }
    if (auto* o = paramObj (it.p)) o->beginChangeGesture();
    dragStart = p;
    dragStartValue = value (it.p);

    switch (it.kind)
    {
        case Kind::sw:
            setValue (it.p, value (it.p) > 0.5f ? 0.0f : 1.0f);
            break;
        case Kind::selector:
        {
            // a click near a position label picks it; on the knob, step to the next position
            int best = -1;
            float bestD = 14.0f;
            for (int k = 0; k < it.labels.size(); ++k)
            {
                const float d = p.getDistanceFrom (selectorLabelPos (it, k));
                if (d < bestD) { bestD = d; best = k; }
            }
            const int n = juce::jmax (1, stepsOf (it.p));
            setIndex (it.p, best >= 0 ? best : (indexOf (it.p) + 1) % (n + 1));
            break;
        }
        case Kind::radio:
            setIndex (it.p, (int) ((p.y - it.y) / it.rowH));
            break;
        default: break;
    }
}

void PequodPanel::mouseDrag (const juce::MouseEvent& e)
{
    if (dragItem < 0) return;
    auto& it = items[(size_t) dragItem];
    if (it.kind != Kind::knob) return;
    const float speed = e.mods.isShiftDown() ? 0.0008f : 0.005f;
    const float dv = (dragStart.y - e.position.y + (e.position.x - dragStart.x) * 0.3f) * speed;
    setValue (it.p, dragStartValue + dv);
}

void PequodPanel::mouseUp (const juce::MouseEvent&)
{
    if (dragItem >= 0)
        if (auto* o = paramObj (items[(size_t) dragItem].p))
            o->endChangeGesture();
    dragItem = -1;
}

void PequodPanel::mouseDoubleClick (const juce::MouseEvent& e)
{
    const int k = hitTest (e.position);
    if (k < 0 || items[(size_t) k].kind != Kind::knob) return;
    if (auto* o = paramObj (items[(size_t) k].p))
    {
        o->beginChangeGesture();
        setValue (items[(size_t) k].p, o->getDefaultValue());
        o->endChangeGesture();
    }
}

void PequodPanel::mouseWheelMove (const juce::MouseEvent& e, const juce::MouseWheelDetails& w)
{
    const int k = hitTest (e.position);
    if (k < 0) return;
    auto& it = items[(size_t) k];
    if (! isActive (it)) return;
    if (it.kind == Kind::knob)
        setValue (it.p, value (it.p) + w.deltaY * (e.mods.isShiftDown() ? 0.01f : 0.05f));
    else if (it.kind == Kind::selector || it.kind == Kind::radio)
        setIndex (it.p, indexOf (it.p) + (w.deltaY > 0 ? -1 : 1) * (it.kind == Kind::radio ? 1 : -1));
}

void PequodPanel::mouseMove (const juce::MouseEvent& e)
{
    lastMouse = e.position;
    const int k = hitTest (e.position);
    setMouseCursor (k >= 0 || presetBox.contains (e.position) || prevBox.contains (e.position) || nextBox.contains (e.position)
                        ? juce::MouseCursor::PointingHandCursor : juce::MouseCursor::NormalCursor);
}

juce::String PequodPanel::getTooltip()
{
    const int k = hitTest (lastMouse);
    if (k < 0) return {};
    auto& it = items[(size_t) k];
    auto t = it.tip;
    if (it.kind == Kind::knob) t = text (it.p) + (t.isNotEmpty() ? "  -  " + t : juce::String());
    return t;
}

//==============================================================================
void PequodPanel::applyPreset (int index)
{
    if (! juce::isPositiveAndBelow (index, (int) presets.size())) return;
    currentPreset = index;
    auto& ps = inst.getParameters();
    for (int p = 0; p < ps.size(); ++p)
    {
        ps[p]->beginChangeGesture();
        ps[p]->setValueNotifyingHost (ps[p]->getDefaultValue());
    }
    for (auto [p, plain] : presets[(size_t) index].values)
        if (juce::isPositiveAndBelow (p, ps.size()))
            ps[p]->setValueNotifyingHost (inst.normalise (p, plain));
    for (auto* p : ps) p->endChangeGesture();
    repaint();
}

void PequodPanel::showPresetMenu()
{
    juce::PopupMenu m;
    for (int k = 0; k < (int) presets.size(); ++k)
        m.addItem (k + 1, presets[(size_t) k].name, true, k == currentPreset);
    juce::Component::SafePointer<PequodPanel> safe (this);
    m.showMenuAsync (juce::PopupMenu::Options().withTargetScreenArea (localAreaToGlobal (presetBox.toNearestInt())),
                     [safe] (int r) { if (safe != nullptr && r > 0) safe->applyPreset (r - 1); });
}

void PequodPanel::timerCallback()
{
    bool dirty = false;
    if (! meters.empty())
    {
        const int n = inst.takeMeters (meters.data(), (int) meters.size());
        for (int k = 0; k < n; ++k)
        {
            // meters fall back gently, like a needle
            const float target = meters[(size_t) k];
            float& shown = shownMeters[(size_t) k];
            const float next = target >= shown ? target : shown + (target - shown) * 0.25f;
            if (std::abs (next - shown) > 1.0e-4f) { shown = next; dirty = true; }
        }
    }
    auto& ps = inst.getParameters();
    for (int p = 0; p < ps.size() && p < (int) lastValues.size(); ++p)
    {
        const float v = ps[p]->getValue();
        if (v != lastValues[(size_t) p]) { lastValues[(size_t) p] = v; dirty = true; }
    }
    for (auto& it : items)
        if (it.kind == Kind::rotor)
        {
            for (int k = 0; k < 2; ++k)
            {
                const int m = k == 0 ? it.horn : it.drum;
                const float speed = juce::isPositiveAndBelow (m, (int) shownMeters.size()) ? shownMeters[(size_t) m] : 0.0f;
                rotorPhase[k] += speed * juce::MathConstants<float>::twoPi / 30.0f;
            }
            dirty = true;
        }
    if (dirty) repaint();
}
