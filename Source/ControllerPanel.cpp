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
        struct Row { juce::String label; juce::Array<juce::Component*> comps; int h = 28; bool heading = false; int labelW = -1; };
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

        int layout (juce::Rectangle<int> area)
        {
            labels.clear(); headings.clear();
            int y = area.getY();
            for (auto& r : rows)
            {
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
            return y - area.getY();
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
/** The controller drawn as it is: one card per board, in the order they are chained. */
class ControllerPanel::BoardView : public juce::Component
{
public:
    explicit BoardView (ControllerManager& m) : mgr (m) {}

    std::function<void (const juce::String&, int)> onSelect;
    juce::String selUid;
    int selCode = -1;

    struct Hit { juce::String uid; int code; juce::Rectangle<float> r; };
    struct Card { const ControllerUnit* unit; juce::Rectangle<float> r; };

    void paint (juce::Graphics& g) override
    {
        g.setColour (Pequod::Colours::well);
        g.fillRoundedRectangle (getLocalBounds().toFloat(), 6.0f);

        hits.clear();
        auto& units = mgr.getInfo().units;
        if (units.empty())
        {
            g.setColour (Pequod::Colours::textDim);
            g.setFont (Pequod::body (15.0f));
            g.drawFittedText ("No boards found on the controller.\nCheck the cables between the boards, then press RESCAN under SETTINGS.",
                              getLocalBounds().reduced (30), juce::Justification::centred, 4);
            return;
        }

        for (auto& card : layoutCards())
            drawCard (g, card);
    }

    void mouseDown (const juce::MouseEvent& e) override
    {
        for (auto& h : hits)
            if (h.r.expanded (6.0f).contains (e.position))
            {
                if (onSelect) onSelect (h.uid, h.code);
                return;
            }
    }

private:
    ControllerManager& mgr;
    std::vector<Hit> hits;

    static float widthFor (const ControllerUnit& u)
    {
        if (u.hasButtons())         return 330.0f;
        if (u.kind == "pedal")      return 140.0f;
        return 200.0f;
    }

    std::vector<Card> layoutCards() const
    {
        std::vector<Card> cards;
        const float pad = 14.0f, h = 168.0f;
        float x = pad, y = pad;
        for (auto& u : mgr.getInfo().units)
        {
            const float w = widthFor (u);
            if (x + w > (float) getWidth() - pad && x > pad) { x = pad; y += h + pad; }
            cards.push_back ({ &u, { x, y, w, h } });
            x += w + pad;
        }
        return cards;
    }

    void drawCard (juce::Graphics& g, const Card& card)
    {
        auto& u = *card.unit;
        auto r = card.r;
        g.setGradientFill (juce::ColourGradient (Pequod::Colours::panelHi, r.getX(), r.getY(), Pequod::Colours::panel, r.getX(), r.getBottom(), false));
        g.fillRoundedRectangle (r, 6.0f);
        g.setColour (u.uid == selUid ? Pequod::Colours::mustardLo : Pequod::Colours::line);
        g.drawRoundedRectangle (r, 6.0f, 1.0f);

        auto head = r.reduced (12.0f, 8.0f).removeFromTop (18.0f);
        g.setColour (Pequod::Colours::mustard);
        g.setFont (Pequod::heading (13.0f));
        g.drawText (mgr.unitName (u.uid).toUpperCase(), head, juce::Justification::centredLeft);
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::body (11.0f));
        g.drawText ("#" + juce::String (u.slot + 1), head, juce::Justification::centredRight);

        auto area = r.reduced (12.0f).withTrimmedTop (26.0f);
        auto codes = ControllerManager::controlCodes (u);
        const float slotW = area.getWidth() / (float) juce::jmax (1, codes.size());
        for (int i = 0; i < codes.size(); ++i)
            drawControl (g, u, codes[i], area.withX (area.getX() + slotW * (float) i).withWidth (slotW));
    }

    void drawControl (juce::Graphics& g, const ControllerUnit& u, int code, juce::Rectangle<float> a)
    {
        const bool selected = u.uid == selUid && code == selCode;
        const bool down = mgr.isPressed (u.uid, code);
        const int lay = mgr.getLayer();
        auto labelArea = a.removeFromBottom (32.0f);
        juce::Rectangle<float> hit;

        if (code == Mapping::ctrlPedal)
        {
            auto bar = a.withSizeKeepingCentre (34.0f, a.getHeight() - 6.0f);
            g.setColour (Pequod::Colours::rail);
            g.fillRoundedRectangle (bar, 6.0f);
            const float v = mgr.pedalValue (u.uid);
            g.setColour (Pequod::Colours::mustard.withAlpha (0.85f));
            g.fillRoundedRectangle (bar.reduced (4.0f).withTrimmedTop ((1.0f - v) * (bar.getHeight() - 8.0f)), 4.0f);
            for (int k = 1; k < 8; ++k)
            {
                g.setColour (juce::Colours::black.withAlpha (0.35f));
                g.fillRect (bar.getX() + 6.0f, bar.getY() + bar.getHeight() * (float) k / 8.0f, bar.getWidth() - 12.0f, 1.0f);
            }
            hit = bar;
        }
        else if (code == Mapping::ctrlEncoderTurn)
        {
            auto k = a.withSizeKeepingCentre (64.0f, 64.0f);
            g.setGradientFill (juce::ColourGradient (juce::Colour (0xff4a4a4a), k.getX(), k.getY(), juce::Colour (0xff141414), k.getRight(), k.getBottom(), false));
            g.fillEllipse (k);
            g.setColour (Pequod::Colours::line);
            g.drawEllipse (k, 1.0f);
            g.setColour (Pequod::Colours::mustard);
            g.drawLine (k.getCentreX(), k.getY() + 8.0f, k.getCentreX(), k.getCentreY() - 6.0f, 3.0f);
            juce::Path arc;
            arc.addCentredArc (k.getCentreX(), k.getCentreY(), 38.0f, 38.0f, 0.0f, -2.2f, 2.2f, true);
            g.setColour (Pequod::Colours::textDim.withAlpha (0.5f));
            g.strokePath (arc, juce::PathStrokeType (1.5f));
            hit = k;
        }
        else
        {
            // footswitch: metal cap with an LED ring
            auto c = a.withSizeKeepingCentre (58.0f, 58.0f);
            const bool hasLed = ControllerManager::hasLed (u, code);
            const auto col = mgr.colourOf (u.uid, code);
            const bool lit = mgr.isLit (u.uid, code);
            if (hasLed)
            {
                if (lit)
                {
                    g.setColour (col.withAlpha (0.25f));
                    g.fillEllipse (c.expanded (7.0f));
                }
                g.setColour (lit ? col : col.withAlpha (0.22f));
                g.drawEllipse (c.reduced (2.0f), 4.0f);
            }
            auto cap = c.reduced (9.0f);
            if (down) cap = cap.reduced (2.0f);
            g.setGradientFill (juce::ColourGradient (juce::Colour (down ? 0xff9a9a9a : 0xff707070), cap.getX(), cap.getY(),
                                                     juce::Colour (0xff1c1c1c), cap.getRight(), cap.getBottom(), false));
            g.fillEllipse (cap);
            g.setColour (juce::Colours::black.withAlpha (0.6f));
            g.drawEllipse (cap, 1.0f);
            hit = c;
        }

        if (selected)
        {
            g.setColour (Pequod::Colours::mustard);
            g.drawRoundedRectangle (hit.expanded (6.0f), 8.0f, 2.0f);
        }

        auto label = mgr.shortLabel (u.uid, code, lay);
        g.setColour (label == "-" ? Pequod::Colours::textDim : Pequod::Colours::text);
        g.setFont (Pequod::heading (12.0f));
        g.drawFittedText (label, labelArea.removeFromTop (16.0f).toNearestInt(), juce::Justification::centred, 1);
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::body (10.5f));
        g.drawText (ControllerManager::controlName (u, code), labelArea, juce::Justification::centredTop);

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
/** What the selected control does. */
class ControllerPanel::Inspector : public juce::Component
{
public:
    Inspector (ControllerManager& m, HostEngine& e, ControlSurface& c) : mgr (m), engine (e), control (c)
    {
        for (auto* comp : std::initializer_list<juce::Component*> { &title, &sub, &identifyButton, &unitName, &modeBox, &functionButton,
                                                                   &latchButton, &momentaryButton, &midiTypeBox, &numberSlider, &channelBox,
                                                                   &layerActionBox, &layerNumBox, &swatches, &ledModeBox, &meter, &everyPresetButton, &thisPresetButton, &useGlobalButton,
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

        styleEditor (unitName);
        unitName.setTooltip ("Name of this board");
        unitName.onReturnKey = [this] { commitUnitName(); unitName.unfocusAllComponents(); };
        unitName.onFocusLost = [this] { commitUnitName(); };

        for (auto* b : { &modeBox, &midiTypeBox, &channelBox, &layerActionBox, &layerNumBox, &ledModeBox }) styleCombo (*b);
        modeBox.onChange = [this] { modeChosen(); };

        functionButton.setColour (juce::TextButton::buttonColourId, Pequod::Colours::well);
        functionButton.onClick = [this] { showFunctionMenu (-1, &functionButton); };
        addFunctionButton.setColour (juce::TextButton::buttonColourId, Pequod::Colours::panel);
        addFunctionButton.setTooltip ("One press can do several things, e.g. switch two effects, or an effect and a plugin knob");
        addFunctionButton.onClick = [this] { showFunctionMenu (-1, &addFunctionButton); };
        addChildComponent (addFunctionButton);
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
        everyPresetButton.setTooltip ("This switch does the same in every preset");
        thisPresetButton.setTooltip ("Give this switch a different job in this preset only. The other presets keep theirs.");
        everyPresetButton.onClick = [this] { setScope (false); };
        thisPresetButton.onClick  = [this] { setScope (true); };
        useGlobalButton.onClick = [this] { mgr.unassign (uid, code, mgr.getLayer()); refresh(); };
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
            return;
        }

        const int lay = mgr.getLayer();
        const auto mode = mgr.modeOf (uid, code, lay);
        const bool sw = ControllerManager::isSwitch (code), pedal = code == Mapping::ctrlPedal, turn = code == Mapping::ctrlEncoderTurn;

        title.setText (ControllerManager::controlName (*u, code).toUpperCase(), juce::dontSendNotification);
        sub.setText (mgr.getNumLayers() > 1 ? "Layer " + juce::String (lay + 1) + " - each layer can do something different"
                                            : juce::String ("Add layers under SETTINGS to give every switch more jobs"), juce::dontSendNotification);
        form.wide ({ &title, &identifyButton }, 30);
        form.wide ({ &sub }, 16);
        if (! unitName.hasKeyboardFocus (true)) unitName.setText (mgr.unitName (uid), false);
        form.row ("Board", { &unitName });

        // What it does
        form.heading ("WHAT IT DOES");
        modeBox.clear (juce::dontSendNotification);
        modeBox.addItem ("Control PEQUOD", 1);
        modeBox.addItem ("Send MIDI to other gear", 2);
        if (sw || turn) modeBox.addItem ("Change layer", 3);
        modeBox.addItem ("Nothing", 4);
        const int modeId = mode == "pequod" ? 1 : mode == "none" ? 4 : mode.startsWith ("layer") ? 3 : 2;
        modeBox.setSelectedId (modeId, juce::dontSendNotification);
        form.wide ({ &modeBox });

        if (modeId == 1)
        {
            auto all = mgr.mappingsFor (uid, code, lay);
            auto* m = all.empty() ? nullptr : all.front();
            auto* global = control.findControllerMapping (uid, code, lay, 0);
            if (all.empty())
            {
                functionButton.setButtonText ("Choose what it does...");
                form.row ("Function", { &functionButton }, 32);
            }
            else
            {
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
                    form.row (i == 0 ? juce::String (all.size() > 1 ? "Functions" : "Function") : juce::String(),
                              { funcButtons[i], removeButtons[i] }, 32);
                }
                form.row ({}, { &addFunctionButton }, 26);
            }
            if (m != nullptr)
            {
                everyPresetButton.setToggleState (! m->perPreset, juce::dontSendNotification);
                thisPresetButton.setToggleState (m->perPreset, juce::dontSendNotification);
                form.row ("Applies to", { &everyPresetButton, &thisPresetButton });
                if (m->perPreset && global != nullptr)
                {
                    useGlobalButton.setButtonText ("USE THE EVERY-PRESET JOB HERE");
                    useGlobalButton.setTooltip ("In every other preset this switch does: " + describe (*global));
                    form.row ({}, { &useGlobalButton });
                }
                if (! m->perPreset && m->isRackMapping() && m->slotUid == 0)
                    showHint (m->pluginName + " isn't in this preset, so this switch does nothing here.");
            }
            const bool oneShot = ! all.empty() && std::all_of (all.begin(), all.end(), [] (const Mapping* f)
            {
                return f->target == Mapping::Target::presetNext || f->target == Mapping::Target::presetPrev
                    || f->target == Mapping::Target::presetSelect || f->target == Mapping::Target::bankNext
                    || f->target == Mapping::Target::bankPrev;
            });
            if (sw && ! oneShot)
                showLatch (mgr.isLatching (uid, code, lay));
            if (m != nullptr && ! m->perPreset && (m->target == Mapping::Target::bypass || m->target == Mapping::Target::param))
                showHint ("Follows " + m->pluginName + (m->pluginOccurrence > 0 ? " #" + juce::String (m->pluginOccurrence + 1) : juce::String())
                          + " in every preset that has it.");
        }
        else if (modeId == 2)
        {
            auto modes = midiModes();
            midiTypeBox.clear (juce::dontSendNotification);
            midiTypeBox.addItemList (midiModeNames(), 1);
            midiTypeBox.setSelectedItemIndex (juce::jmax (0, modes.indexOf (mode)), juce::dontSendNotification);
            form.row ("Message", { &midiTypeBox });
            const bool needsNumber = ! (turn && mode == "pc");
            if (needsNumber)
            {
                numberSlider.setValue ((int) mgr.get (ControllerManager::actionPath (uid, code, lay) + ".num"), juce::dontSendNotification);
                form.row (mode == "note" ? "Note" : mode == "pc" ? "Program" : "CC number", { &numberSlider });
            }
            channelBox.setSelectedId (juce::jlimit (1, 16, (int) mgr.get (ControllerManager::actionPath (uid, code, lay) + ".ch")), juce::dontSendNotification);
            form.row ("Channel", { &channelBox });
            if (sw && (mode == "cc" || mode == "note"))
            {
                showLatch (mgr.isLatching (uid, code, lay));
            }
            showHint (midiOutHint());
        }
        else if (modeId == 3)
        {
            if (turn)
                showHint ("Turning it steps through the layers.");
            else
            {
                static const juce::StringArray modes { "layer_next", "layer_prev", "layer_set", "layer_hold" };
                layerActionBox.setSelectedItemIndex (juce::jmax (0, modes.indexOf (mode)), juce::dontSendNotification);
                form.row ("Action", { &layerActionBox });
                if (mode == "layer_set" || mode == "layer_hold")
                {
                    layerNumBox.setSelectedId (juce::jlimit (1, 4, (int) mgr.get (ControllerManager::actionPath (uid, code, lay) + ".num")), juce::dontSendNotification);
                    form.row ("Layer", { &layerNumBox });
                }
                if (mgr.getNumLayers() < 2) showHint ("There is only one layer. Add more under SETTINGS > Layers.");
            }
        }

        // LED
        if (ControllerManager::hasLed (*u, code))
        {
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
    }

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
    juce::TextEditor unitName;
    juce::ComboBox modeBox, midiTypeBox, channelBox, layerActionBox, layerNumBox, ledModeBox;
    juce::ToggleButton invertToggle;
    juce::TextButton latchButton { "LATCHING" }, momentaryButton { "MOMENTARY" };
    juce::Slider numberSlider;
    SwatchRow swatches;
    juce::TextButton everyPresetButton { "EVERY PRESET" }, thisPresetButton { "THIS PRESET" }, useGlobalButton;
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

        const bool pequodMode = mgr.modeOf (uid, code, lay) == "pequod";
        if (pequodMode)
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
            if (mgr.modeOf (uid, code, lay) == "pequod") mgr.setUseEffectColour (uid, code, lay, false);
            mgr.set (ControllerManager::controlPath (uid, code) + ".color", col);
        }
        refresh();
    }

    void setScope (bool perPreset)
    {
        const int lay = mgr.getLayer();
        auto* m = mgr.mappingFor (uid, code, lay);
        if (m == nullptr || m->perPreset == perPreset) { refresh(); return; }
        if (perPreset)
        {
            std::vector<Mapping> copies;  // the other presets keep the every-preset functions
            for (auto* f : mgr.mappingsFor (uid, code, lay)) copies.push_back (*f);
            for (auto& c : copies) { c.perPreset = true; control.setControllerMapping (c); }
            if (mgr.onStatus) mgr.onStatus ("This preset now has its own job for this switch - change it above");
        }
        else
            control.setControllerScope (uid, code, lay, false);
        refresh();
    }

    void showHint (const juce::String& s)
    {
        hint.setText (s, juce::dontSendNotification);
        form.wide ({ &hint }, 54);
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

    void commitUnitName()
    {
        auto n = unitName.getText().trim();
        if (n.isNotEmpty() && n != mgr.unitName (uid)) mgr.set ("units." + uid + ".name", n.substring (0, 20));
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
            juce::MessageManager::callAsync ([safe = juce::Component::SafePointer<Inspector> (this)] { if (safe) safe->showFunctionMenu (-1, &safe->functionButton); });
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
            default:                            return control.describeTarget (m);
        }
    }

    void removeFunctionAt (int i)
    {
        juce::MessageManager::callAsync ([safe = juce::Component::SafePointer<Inspector> (this), i]
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

            menu.addSectionHeader ("EFFECTS");
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
            if (! splits.empty()) menu.addSubMenu ("Split mix", splitMenu);
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
                                              : juce::String ("Load a new PEQUOD_FC.bin onto the controller"), juce::dontSendNotification);
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
        auto bundled = juce::File::getSpecialLocation (juce::File::currentExecutableFile).getSiblingFile ("Controller firmware");
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

    addChildComponent (*board);
    addChildComponent (*inspector);
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

    quickSetupButton.setTooltip ("Give every switch that isn't set up yet a useful job: the first effects of this preset on/off, next preset, preset 1-4 on a second board, tuner...");
    quickSetupButton.onClick = [this]
    {
        if (! mgr.quickSetup() && mgr.onStatus) mgr.onStatus ("Every control already has a job");
        inspector->refresh();
    };
    addChildComponent (quickSetupButton);

    for (int i = 0; i < 4; ++i)
    {
        auto* b = layerButtons.add (new juce::TextButton ("LAYER " + juce::String (i + 1)));
        b->setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
        b->setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
        b->onClick = [this, i] { mgr.selectLayer (i); };
        addChildComponent (b);
    }

    board->onSelect = [this] (const juce::String& uid, int code) { select (uid, code); };
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
    board->setVisible (conn && ! showingSettings);
    inspector->setVisible (conn && ! showingSettings);
    quickSetupButton.setVisible (conn && ! showingSettings);
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
        auto tip = quickSetupButton.getBounds().withLeft (layerButtons[0]->isVisible() ? layerButtons[mgr.getNumLayers() - 1]->getRight() + 16 : 18);
        g.setFont (Pequod::body (13.0f));
        g.drawText ("Press any switch on the controller to pick it here", tip.withTrimmedRight (quickSetupButton.getWidth() + 16),
                    juce::Justification::centredRight);
    }
}

void ControllerPanel::resized()
{
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
    quickSetupButton.setBounds (bar.removeFromRight (130));
    for (auto* b : layerButtons)
        if (b->isVisible()) { b->setBounds (bar.removeFromLeft (84)); bar.removeFromLeft (4); }
    r.removeFromTop (10);

    auto right = r.removeFromRight (juce::jlimit (320, 420, r.getWidth() * 2 / 5));
    r.removeFromRight (12);
    inspector->setBounds (right);
    board->setBounds (r);
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
    if (! mgr.isConnected())
    {
        if (++idleTicks % 15 == 0) repaint();     // keep the list of checked ports current
        return;
    }
    inspector->tick();
    board->repaint();
}

//==============================================================================
ControllerWindow::ControllerWindow (ControllerManager& m, HostEngine& e, ControlSurface& c)
    : DocumentWindow ("PEQUOD - Foot controller", Pequod::Colours::bg, juce::DocumentWindow::closeButton | juce::DocumentWindow::minimiseButton)
{
    setUsingNativeTitleBar (true);
    panel = new ControllerPanel (m, e, c);
    panel->setSize (1040, 680);
    setContentOwned (panel, true);
    setResizable (true, false);
    setResizeLimits (820, 520, 2400, 1600);
    centreWithSize (getWidth(), getHeight());
}
