#include "ModifierRow.h"

namespace
{
    using Modifiers::Slot;

    void styleButton (juce::TextButton& b, bool small = true)
    {
        b.setColour (juce::TextButton::buttonColourId, Pequod::Colours::well);
        b.setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
        b.setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
        juce::ignoreUnused (small);
    }

    juce::String msText (float ms)
    {
        return ms >= 1000.0f ? juce::String (ms / 1000.0f, 2) + " s" : juce::String (juce::roundToInt (ms)) + " ms";
    }

    // Times on knobs: 1 ms - 5 s, exponential.
    double msFromNorm (double v)  { return 1.0 * std::pow (5000.0, juce::jlimit (0.0, 1.0, v)); }
    double normFromMs (double ms) { return std::log (juce::jlimit (1.0, 5000.0, ms)) / std::log (5000.0); }

    //==========================================================================
    /** A small knob with its name above and its value below. */
    class Knob : public juce::Component
    {
    public:
        Knob (const juce::String& name, std::function<double()> get, std::function<void (double)> set,
              std::function<juce::String()> text)
            : label (name), getter (std::move (get)), setter (std::move (set)), texter (std::move (text))
        {
            slider.setSliderStyle (juce::Slider::RotaryHorizontalVerticalDrag);
            slider.setTextBoxStyle (juce::Slider::NoTextBox, false, 0, 0);
            slider.setRange (0.0, 1.0);
            slider.setDoubleClickReturnValue (false, 0.0);
            slider.setValue (getter(), juce::dontSendNotification);
            slider.onValueChange = [this] { setter (slider.getValue()); repaint(); };
            addAndMakeVisible (slider);
        }

        void refresh()
        {
            const double v = getter();
            if (std::abs (v - slider.getValue()) > 1.0e-4 && ! slider.isMouseButtonDown())
                slider.setValue (v, juce::dontSendNotification);
            const auto t = texter();
            if (t != shown) { shown = t; repaint(); }
        }

        void paint (juce::Graphics& g) override
        {
            g.setColour (Pequod::Colours::textDim);
            g.setFont (Pequod::heading (9.5f));
            g.drawText (label, getLocalBounds().removeFromTop (12), juce::Justification::centred);
            g.setColour (Pequod::Colours::text);
            g.setFont (Pequod::heading (10.0f, false));
            g.drawText (shown, getLocalBounds().removeFromBottom (12), juce::Justification::centred);
        }

        void resized() override { slider.setBounds (getLocalBounds().reduced (0, 12)); }

        juce::Slider slider;

    private:
        juce::String label, shown;
        std::function<double()> getter;
        std::function<void (double)> setter;
        std::function<juce::String()> texter;
    };

    //==========================================================================
    /** FROM - TO for one assignment: two handles; the filled part is the sweep. */
    class RangeBar : public juce::Component, public juce::SettableTooltipClient
    {
    public:
        explicit RangeBar (Modifiers::Assignment& a, juce::Colour c, std::function<float()> live)
            : assignment (a), colour (c), liveValue (std::move (live)) {}

        void paint (juce::Graphics& g) override
        {
            auto r = getLocalBounds().toFloat().reduced (5.0f, 4.0f);
            g.setColour (Pequod::Colours::rail);
            g.fillRoundedRectangle (r, 3.0f);
            const float f = assignment.from.load(), t = assignment.to.load();
            const float x1 = r.getX() + r.getWidth() * juce::jmin (f, t), x2 = r.getX() + r.getWidth() * juce::jmax (f, t);
            g.setColour (colour.withAlpha (0.35f));
            g.fillRect (juce::Rectangle<float> (x1, r.getY(), juce::jmax (1.0f, x2 - x1), r.getHeight()));
            // where the knob is now
            const float now = f + (t - f) * liveValue();
            g.setColour (colour);
            g.fillRect (juce::Rectangle<float> (r.getX() + r.getWidth() * now - 1.0f, r.getY(), 2.0f, r.getHeight()));
            // handles: FROM hollow, TO filled
            auto handle = [&] (float v, bool filled)
            {
                auto hb = juce::Rectangle<float> (7.0f, r.getHeight() + 4.0f).withCentre ({ r.getX() + r.getWidth() * v, r.getCentreY() });
                g.setColour (Pequod::Colours::text);
                if (filled) g.fillRoundedRectangle (hb, 2.0f); else g.drawRoundedRectangle (hb.reduced (0.5f), 2.0f, 1.2f);
            };
            handle (f, false);
            handle (t, true);
        }

        void mouseDown (const juce::MouseEvent& e) override
        {
            const float v = toValue (e.x);
            dragging = std::abs (v - assignment.from.load()) <= std::abs (v - assignment.to.load()) ? 0 : 1;
            mouseDrag (e);
        }
        void mouseDrag (const juce::MouseEvent& e) override
        {
            (dragging == 0 ? assignment.from : assignment.to) = toValue (e.x);
            if (onChange) onChange();
            repaint();
        }
        void mouseDoubleClick (const juce::MouseEvent&) override
        {
            const float f = assignment.from.load();         // swap direction
            assignment.from = assignment.to.load();
            assignment.to = f;
            if (onChange) onChange();
            repaint();
        }

        std::function<void()> onChange;

    private:
        Modifiers::Assignment& assignment;
        juce::Colour colour;
        std::function<float()> liveValue;
        int dragging = 0;

        float toValue (int x) const
        {
            auto r = getLocalBounds().toFloat().reduced (5.0f, 4.0f);
            return juce::jlimit (0.0f, 1.0f, ((float) x - r.getX()) / r.getWidth());
        }
    };
}

//==============================================================================
class ModifierRow::Module : public juce::Component
{
public:
    Module (HostEngine& e, int idx) : engine (e), index (idx)
    {
        auto& s = slot();
        colour = colourFor (s.type.load());

        for (auto* b : { &onButton, &learnButton, &removeButton, &leftButton, &rightButton, &invertButton,
                         &shapeButton, &syncButton, &triggerButton, &holdButton, &fireButton })
            styleButton (*b);
        onButton.setClickingTogglesState (true);
        onButton.setToggleState (s.on.load(), juce::dontSendNotification);
        onButton.onClick = [this] { slot().on = onButton.getToggleState(); engine.markModified(); };
        onButton.setTooltip ("Turn this modifier on / off (off: the knobs go back to FROM)");
        learnButton.setTooltip ("Click, then move a knob on any effect: this modifier will move it");
        learnButton.onClick = [this]
        {
            engine.armModifierLearn (engine.getModifierLearn() == index ? -1 : index);
        };
        removeButton.setTooltip ("Remove this modifier");
        removeButton.onClick = [this]
        {
            auto& eng = engine;
            const int i = index;
            juce::MessageManager::callAsync ([&eng, i] { eng.removeModifier (i); });   // this module is deleted by it
        };
        leftButton.setTooltip ("Move left");
        rightButton.setTooltip ("Move right");
        leftButton.onClick  = [this] { auto& eng = engine; const int i = index; juce::MessageManager::callAsync ([&eng, i] { eng.moveModifier (i, i - 1); }); };
        rightButton.onClick = [this] { auto& eng = engine; const int i = index; juce::MessageManager::callAsync ([&eng, i] { eng.moveModifier (i, i + 1); }); };
        invertButton.setClickingTogglesState (true);
        invertButton.setToggleState (s.invert.load(), juce::dontSendNotification);
        invertButton.setTooltip ("Turn it upside down");
        invertButton.onClick = [this] { slot().invert = invertButton.getToggleState(); engine.markModified(); };
        for (auto* b : { &onButton, &learnButton, &removeButton, &leftButton, &rightButton, &invertButton })
            addAndMakeVisible (b);

        auto mark = [this] { engine.markModified(); };
        auto amount = new Knob ("AMOUNT", [this] { return (double) slot().amount.load(); },
                                [this, mark] (double v) { slot().amount = (float) v; mark(); },
                                [this] { return juce::String (juce::roundToInt (slot().amount.load() * 100.0f)) + "%"; });

        switch ((Modifiers::Type) s.type.load())
        {
            case Modifiers::Type::lfo:
            {
                shapeButton.setTooltip ("Wave shape (click to change)");
                shapeButton.onClick = [this]
                {
                    juce::PopupMenu m;
                    for (int k = 0; k < Modifiers::numShapes; ++k)
                        m.addItem (Modifiers::shapeName ((Modifiers::Shape) k), true, slot().shape.load() == k,
                                   [this, k] { slot().shape = k; engine.markModified(); updateButtons(); });
                    m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (&shapeButton));
                };
                syncButton.setClickingTogglesState (true);
                syncButton.setToggleState (s.sync.load(), juce::dontSendNotification);
                syncButton.setTooltip ("SYNC: follows the tempo (1/4, 1/8...). Off: free rate in Hz");
                syncButton.onClick = [this] { slot().sync = syncButton.getToggleState(); engine.markModified(); };
                addAndMakeVisible (shapeButton);
                addAndMakeVisible (syncButton);
                knobs.add (new Knob ("RATE", [this] { return rateNorm(); }, [this, mark] (double v) { setRateNorm (v); mark(); },
                                     [this] { return rateText(); }));
                knobs.add (new Knob ("PHASE", [this] { return (double) slot().phase.load(); },
                                     [this, mark] (double v) { slot().phase = (float) v; mark(); },
                                     [this] { return juce::String (juce::roundToInt (slot().phase.load() * 360.0f)) + juce::String (juce::CharPointer_UTF8 ("\xc2\xb0")); }));
                knobs.add (amount);
                break;
            }
            case Modifiers::Type::follower:
                knobs.add (new Knob ("SENS", [this] { return slot().sensitivity.load() / 40.0; },
                                     [this, mark] (double v) { slot().sensitivity = (float) (v * 40.0); mark(); },
                                     [this] { return "+" + juce::String (juce::roundToInt (slot().sensitivity.load())) + " dB"; }));
                knobs.add (new Knob ("ATTACK", [this] { return normFromMs (slot().attackMs.load()); },
                                     [this, mark] (double v) { slot().attackMs = (float) msFromNorm (v); mark(); },
                                     [this] { return msText (slot().attackMs.load()); }));
                knobs.add (new Knob ("RELEASE", [this] { return normFromMs (slot().releaseMs.load()); },
                                     [this, mark] (double v) { slot().releaseMs = (float) msFromNorm (v); mark(); },
                                     [this] { return msText (slot().releaseMs.load()); }));
                knobs.add (amount);
                break;

            case Modifiers::Type::envelope:
            {
                triggerButton.setTooltip ("What starts it: a footswitch / MIDI / the FIRE button, your picking, or MIDI notes");
                triggerButton.onClick = [this]
                {
                    juce::PopupMenu m;
                    const char* names[] = { "FOOTSWITCH / FIRE BUTTON", "PICKING (each note you play)", "MIDI NOTES" };
                    for (int k = 0; k < 3; ++k)
                        m.addItem (names[k], true, slot().trigger.load() == k, [this, k] { slot().trigger = k; engine.markModified(); updateButtons(); resized(); });
                    m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (&triggerButton));
                };
                holdButton.setClickingTogglesState (true);
                holdButton.setToggleState (s.holdMode.load(), juce::dontSendNotification);
                holdButton.setTooltip ("HOLD: stays at SUSTAIN while the switch / note / string is held. Off: one shot");
                holdButton.onClick = [this] { slot().holdMode = holdButton.getToggleState(); engine.markModified(); };
                fireButton.setTooltip ("Fire it now (to try it)");
                fireButton.onClick = [this] { ++slot().pressCount; };
                fireButton.onStateChange = [this] { slot().held = fireButton.isDown(); };
                addAndMakeVisible (triggerButton);
                addAndMakeVisible (holdButton);
                addAndMakeVisible (fireButton);
                auto timeKnob = [this, mark] (const char* name, std::atomic<float> Slot::* member)
                {
                    return new Knob (name, [this, member] { return normFromMs ((slot().*member).load()); },
                                     [this, member, mark] (double v) { (slot().*member) = (float) msFromNorm (v); mark(); },
                                     [this, member] { return msText ((slot().*member).load()); });
                };
                knobs.add (timeKnob ("A", &Slot::aMs));
                knobs.add (timeKnob ("D", &Slot::dMs));
                knobs.add (new Knob ("S", [this] { return (double) slot().sustain.load(); },
                                     [this, mark] (double v) { slot().sustain = (float) v; mark(); },
                                     [this] { return juce::String (juce::roundToInt (slot().sustain.load() * 100.0f)) + "%"; }));
                knobs.add (timeKnob ("R", &Slot::rMs));
                knobs.add (new Knob ("PICK", [this] { return (slot().pickThresholdDb.load() + 70.0) / 60.0; },
                                     [this, mark] (double v) { slot().pickThresholdDb = (float) (-70.0 + v * 60.0); mark(); },
                                     [this] { return juce::String (juce::roundToInt (slot().pickThresholdDb.load())) + " dB"; }));
                knobs.add (amount);
                break;
            }
            case Modifiers::Type::sequencer:
                knobs.add (new Knob ("STEPS", [this] { return (slot().length.load() - 1) / 15.0; },
                                     [this, mark] (double v) { slot().length = 1 + juce::roundToInt (v * 15.0); mark(); },
                                     [this] { return juce::String (slot().length.load()); }));
                knobs.add (new Knob ("RATE", [this] { return rateNorm(); }, [this, mark] (double v) { setRateNorm (v); mark(); },
                                     [this] { return rateText(); }));
                knobs.add (new Knob ("GLIDE", [this] { return (double) slot().glide.load(); },
                                     [this, mark] (double v) { slot().glide = (float) v; mark(); },
                                     [this] { return juce::String (juce::roundToInt (slot().glide.load() * 100.0f)) + "%"; }));
                knobs.add (amount);
                break;
        }
        for (auto* k : knobs)
        {
            k->slider.setColour (juce::Slider::rotarySliderFillColourId, colour);
            addAndMakeVisible (k);
        }
        rebuildAssignments();
        updateButtons();
    }

    Slot& slot() { return engine.modifier (index); }
    int width() { return slot().type.load() == (int) Modifiers::Type::envelope ? 360 : 300; }

    void tick()
    {
        auto& s = slot();
        history[(size_t) historyPos] = s.out.load();
        historyPos = (historyPos + 1) % (int) history.size();
        for (auto* k : knobs) k->refresh();
        if (onButton.getToggleState() != s.on.load()) onButton.setToggleState (s.on.load(), juce::dontSendNotification);
        const bool learning = engine.getModifierLearn() == index;
        if (learnButton.getToggleState() != learning)
        {
            learnButton.setToggleState (learning, juce::dontSendNotification);
            learnButton.setButtonText (learning ? "MOVE A KNOB..." : "LEARN");
            resized();
        }
        if ((int) engine.modifierInfo (index).assignments.size() != rangeBars.size()) { rebuildAssignments(); resized(); }
        for (auto* rb : rangeBars) rb->repaint();
        if (s.fired.exchange (false)) flash = 6;
        if (flash > 0) --flash;
        repaint (scopeArea.expanded (2));
    }

    void paint (juce::Graphics& g) override
    {
        auto r = getLocalBounds().toFloat();
        g.setColour (Pequod::Colours::panel);
        g.fillRoundedRectangle (r, 4.0f);
        g.setColour (colour.withAlpha (engine.getModifierLearn() == index ? 1.0f : 0.5f));
        g.drawRoundedRectangle (r.reduced (0.5f), 4.0f, engine.getModifierLearn() == index ? 2.0f : 1.0f);
        g.fillRect (r.removeFromTop (3.0f).reduced (4.0f, 0.0f));

        g.setColour (colour);
        g.setFont (Pequod::heading (13.0f));
        g.drawText (engine.modifierName (index), titleArea, juce::Justification::centredLeft);

        paintScope (g);

        // assignment names
        g.setFont (Pequod::body (11.5f));
        const auto& list = engine.modifierInfo (index).assignments;
        if (list.empty())
        {
            g.setColour (Pequod::Colours::textDim);
            g.drawFittedText ("Click LEARN, then move a knob on any effect.", assignArea, juce::Justification::centred, 2);
        }
        for (int k = 0; k < rangeBars.size() && k < (int) list.size(); ++k)
        {
            g.setColour (Pequod::Colours::text);
            g.drawText (engine.describeAssignment (*list[(size_t) k]), nameAreas[k], juce::Justification::centredLeft, true);
        }
        if (! assignOverflow.isEmpty())
        {
            g.setColour (Pequod::Colours::textDim);
            g.drawText ("+ " + juce::String ((int) list.size() - rangeBars.size()) + " more", assignOverflow, juce::Justification::centredLeft);
        }
    }

    void resized() override
    {
        auto r = getLocalBounds().reduced (8, 6);
        r.removeFromTop (2);
        auto top = r.removeFromTop (22);
        removeButton.setBounds (top.removeFromRight (22));
        top.removeFromRight (2);
        rightButton.setBounds (top.removeFromRight (20));
        leftButton.setBounds (top.removeFromRight (20));
        top.removeFromRight (4);
        learnButton.setBounds (top.removeFromRight (learnButton.getToggleState() ? 110 : 54));
        top.removeFromRight (4);
        onButton.setBounds (top.removeFromRight (36));
        titleArea = top;
        r.removeFromTop (6);

        // scope / steps + type buttons
        auto scopeRow = r.removeFromTop (44);
        auto side = scopeRow.removeFromRight (78);
        scopeRow.removeFromRight (6);
        scopeArea = scopeRow;
        auto sideRow = [&side] { auto b = side.removeFromTop (20); side.removeFromTop (4); return b; };
        const auto type = (Modifiers::Type) slot().type.load();
        if (type == Modifiers::Type::lfo)          { shapeButton.setBounds (sideRow()); syncButton.setBounds (sideRow()); }
        else if (type == Modifiers::Type::envelope) { triggerButton.setBounds (sideRow()); auto b = sideRow(); holdButton.setBounds (b.removeFromLeft (37)); fireButton.setBounds (b.withTrimmedLeft (4)); }
        else                                        { invertButton.setBounds (sideRow()); }
        if (type == Modifiers::Type::lfo || type == Modifiers::Type::envelope)
            invertButton.setVisible (false);
        r.removeFromTop (4);

        auto knobRow = r.removeFromTop (62);
        const int kw = juce::jmin (56, knobRow.getWidth() / juce::jmax (1, knobs.size()));
        for (auto* k : knobs) k->setBounds (knobRow.removeFromLeft (kw));
        r.removeFromTop (6);

        assignArea = r;
        nameAreas.clear();
        assignOverflow = {};
        for (int k = 0; k < rangeBars.size(); ++k)
        {
            auto row = r.removeFromTop (20);
            removeAssign[k]->setBounds (row.removeFromRight (18));
            row.removeFromRight (2);
            rangeBars[k]->setBounds (row.removeFromRight (juce::jmax (80, row.getWidth() / 2)));
            nameAreas.add (row);
        }
        if ((int) engine.modifierInfo (index).assignments.size() > rangeBars.size())
            assignOverflow = r.removeFromTop (16);
    }

    void mouseDown (const juce::MouseEvent& e) override
    {
        if (slot().type.load() == (int) Modifiers::Type::sequencer && scopeArea.contains (e.getPosition())) drawStep (e);
    }
    void mouseDrag (const juce::MouseEvent& e) override
    {
        if (slot().type.load() == (int) Modifiers::Type::sequencer && scopeArea.expanded (0, 20).contains (e.getPosition())) drawStep (e);
    }

private:
    HostEngine& engine;
    int index;
    juce::Colour colour;
    juce::TextButton onButton { "ON" }, learnButton { "LEARN" }, removeButton { "X" },
                     leftButton { "<" }, rightButton { ">" }, invertButton { "INVERT" },
                     shapeButton { "SINE" }, syncButton { "SYNC" }, triggerButton { "PICKING" }, holdButton { "HOLD" }, fireButton { "FIRE" };
    juce::OwnedArray<Knob> knobs;
    juce::OwnedArray<RangeBar> rangeBars;
    juce::OwnedArray<juce::TextButton> removeAssign;
    juce::Rectangle<int> titleArea, scopeArea, assignArea, assignOverflow;
    juce::Array<juce::Rectangle<int>> nameAreas;
    std::array<float, 120> history {};
    int historyPos = 0, flash = 0;

    void updateButtons()
    {
        shapeButton.setButtonText (Modifiers::shapeName ((Modifiers::Shape) slot().shape.load()));
        const char* trig[] = { "SWITCH", "PICKING", "MIDI NOTES" };
        triggerButton.setButtonText (trig[juce::jlimit (0, 2, slot().trigger.load())]);
    }

    double rateNorm()
    {
        auto& s = slot();
        if (s.sync || s.type.load() == (int) Modifiers::Type::sequencer)
            return s.division.load() / (double) juce::jmax (1, (int) Modifiers::divisions().size() - 1);
        return Modifiers::normFromRate (s.rateHz.load());
    }
    void setRateNorm (double v) { ControlSurfaceRate (slot(), (float) v); }
    static void ControlSurfaceRate (Slot& s, float v)
    {
        if (s.sync || s.type.load() == (int) Modifiers::Type::sequencer)
            s.division = juce::roundToInt (v * (float) ((int) Modifiers::divisions().size() - 1));
        else
            s.rateHz = (float) Modifiers::rateFromNorm (v);
    }
    juce::String rateText()
    {
        auto& s = slot();
        if (s.sync || s.type.load() == (int) Modifiers::Type::sequencer)
            return Modifiers::divisions()[(size_t) juce::jlimit (0, (int) Modifiers::divisions().size() - 1, s.division.load())].label;
        const float hz = s.rateHz.load();
        return juce::String (hz, hz < 1.0f ? 2 : 1) + " Hz";
    }

    void rebuildAssignments()
    {
        rangeBars.clear();
        removeAssign.clear();
        const auto& list = engine.modifierInfo (index).assignments;
        const int shown = juce::jmin ((int) list.size(), 3);
        for (int k = 0; k < shown; ++k)
        {
            auto* rb = rangeBars.add (new RangeBar (*list[(size_t) k], colour, [this] { return slot().out.load() * slot().amount.load(); }));
            rb->onChange = [this] { engine.markModified(); };
            rb->setTooltip ("Where this knob goes: hollow handle = FROM, solid = TO. Drag them; double-click to swap");
            addAndMakeVisible (rb);
            auto* x = removeAssign.add (new juce::TextButton ("x"));
            styleButton (*x);
            x->setTooltip ("Stop moving this knob");
            x->onClick = [this, k] { auto& eng = engine; const int i = index; juce::MessageManager::callAsync ([&eng, i, k] { eng.removeModAssignment (i, k); }); };
            addAndMakeVisible (x);
        }
    }

    void paintScope (juce::Graphics& g)
    {
        auto r = scopeArea.toFloat();
        g.setColour (Pequod::Colours::well);
        g.fillRoundedRectangle (r, 3.0f);
        auto& s = slot();

        if (s.type.load() == (int) Modifiers::Type::sequencer)
        {
            const int count = juce::jlimit (1, Modifiers::maxSteps, s.length.load());
            const float w = r.getWidth() / (float) Modifiers::maxSteps;
            for (int k = 0; k < Modifiers::maxSteps; ++k)
            {
                auto cell = juce::Rectangle<float> (r.getX() + w * (float) k, r.getY(), w, r.getHeight()).reduced (1.5f, 3.0f);
                const float v = s.steps[k].load();
                const bool active = k < count;
                const bool now = k == s.currentStep.load() && s.on.load();
                g.setColour (Pequod::Colours::rail);
                g.fillRect (cell);
                g.setColour (! active ? Pequod::Colours::line : now ? Pequod::Colours::text : colour.withAlpha (0.85f));
                g.fillRect (cell.withTop (cell.getBottom() - cell.getHeight() * v));
            }
            return;
        }

        // rolling trace of the last few seconds
        juce::Path p;
        const int n = (int) history.size();
        for (int k = 0; k < n; ++k)
        {
            const float v = history[(size_t) ((historyPos + k) % n)];
            const float x = r.getX() + 3.0f + (r.getWidth() - 6.0f) * (float) k / (float) (n - 1);
            const float y = r.getBottom() - 3.0f - (r.getHeight() - 6.0f) * v;
            if (k == 0) p.startNewSubPath (x, y); else p.lineTo (x, y);
        }
        g.setColour (colour.withAlpha (s.on.load() ? 1.0f : 0.35f));
        g.strokePath (p, juce::PathStrokeType (1.6f));
        if (flash > 0)
        {
            g.setColour (colour.withAlpha (0.12f * (float) flash));
            g.fillRoundedRectangle (r, 3.0f);
        }
        if (s.type.load() == (int) Modifiers::Type::follower)
        {
            g.setColour (Pequod::Colours::textDim);
            g.setFont (Pequod::heading (9.5f, false));
            g.drawText ("YOUR PLAYING", r.reduced (5.0f, 2.0f), juce::Justification::topLeft);
        }
    }

    void drawStep (const juce::MouseEvent& e)
    {
        auto r = scopeArea.toFloat();
        const int k = juce::jlimit (0, Modifiers::maxSteps - 1, (int) ((e.position.x - r.getX()) / (r.getWidth() / (float) Modifiers::maxSteps)));
        const float v = juce::jlimit (0.0f, 1.0f, (r.getBottom() - 3.0f - e.position.y) / (r.getHeight() - 6.0f));
        slot().steps[k] = v;
        if (k >= slot().length.load()) slot().length = k + 1;
        engine.markModified();
        repaint (scopeArea);
    }
};

//==============================================================================
juce::Colour ModifierRow::colourFor (int type)
{
    switch ((Modifiers::Type) type)
    {
        case Modifiers::Type::lfo:       return Pequod::Colours::mustard;
        case Modifiers::Type::follower:  return juce::Colour (0xffe3a391);   // dusty rose
        case Modifiers::Type::envelope:  return juce::Colour (0xffb4c5a8);   // sage
        case Modifiers::Type::sequencer: return juce::Colour (0xffc29a64);   // caramel
    }
    return Pequod::Colours::mustard;
}

ModifierRow::ModifierRow (HostEngine& e) : engine (e)
{
    for (auto* b : { &showButton, &addLfo, &addFollower, &addEnvelope, &addSteps })
    {
        styleButton (*b);
        addAndMakeVisible (b);
    }
    addLfo.setTooltip ("Add an LFO: moves knobs up and down in time with the tempo (or at its own rate)");
    addFollower.setTooltip ("Add an envelope follower: moves knobs with how hard you play (auto-wah, dynamic drive...)");
    addEnvelope.setTooltip ("Add a triggered envelope: a sweep fired by each note you pick, a footswitch or MIDI");
    addSteps.setTooltip ("Add a step sequencer: a pattern of knob positions in time with the tempo");
    addLfo.onClick      = [this] { add (Modifiers::Type::lfo); };
    addFollower.onClick = [this] { add (Modifiers::Type::follower); };
    addEnvelope.onClick = [this] { add (Modifiers::Type::envelope); };
    addSteps.onClick    = [this] { add (Modifiers::Type::sequencer); };
    showButton.onClick  = [this] { setExpanded (! expanded); };

    viewport.setViewedComponent (&strip, false);
    viewport.setScrollBarsShown (false, true);
    viewport.setScrollBarThickness (8);
    addAndMakeVisible (viewport);
    rebuild();
    startTimerHz (30);
}

ModifierRow::~ModifierRow() { stopTimer(); }

void ModifierRow::setExpanded (bool e)
{
    expanded = e;
    showButton.setButtonText (expanded ? "HIDE" : "SHOW");
    viewport.setVisible (expanded);
    if (onHeightChanged) onHeightChanged();
}

void ModifierRow::add (Modifiers::Type t)
{
    if (engine.addModifier (t) < 0)
        juce::AlertWindow::showMessageBoxAsync (juce::MessageBoxIconType::NoIcon, "Modifiers",
                                                "A preset can have up to " + juce::String (Modifiers::maxModifiers) + " modifiers.");
    if (! expanded) setExpanded (true);
}

void ModifierRow::rebuild()
{
    modules.clear();
    for (int i = 0; i < engine.getNumModifiers(); ++i)
        strip.addAndMakeVisible (modules.add (new Module (engine, i)));
    builtCount = engine.getNumModifiers();
    resized();
    repaint();
}

void ModifierRow::timerCallback()
{
    if (builtCount != engine.getNumModifiers()) { rebuild(); return; }
    if (expanded)
        for (auto* m : modules) m->tick();
}

void ModifierRow::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::bg);
    auto bar = getLocalBounds().removeFromTop (barHeight);
    g.setColour (Pequod::Colours::rail);
    g.fillRect (bar);
    g.setColour (Pequod::Colours::mustard);
    g.setFont (Pequod::heading (13.0f));
    g.drawText ("MODIFIERS", bar.reduced (12, 0), juce::Justification::centredLeft);
    g.setColour (Pequod::Colours::textDim);
    g.setFont (Pequod::body (12.0f));
    const int n = engine.getNumModifiers();
    juce::String hint = n == 0 ? "LFOs, envelopes and step sequencers that move effect knobs. Add one:"
                               : juce::String (n) + (n == 1 ? " in this preset" : " in this preset");
    if (engine.getModifierLearn() >= 0) { hint = "LEARN: move a knob on any effect (click LEARN again to stop)"; g.setColour (Pequod::Colours::mustard); }
    g.drawText (hint, bar.withTrimmedLeft (104).withRight (addLfo.getX() - 8), juce::Justification::centredRight, true);
    g.setColour (Pequod::Colours::line);
    g.fillRect (getLocalBounds().removeFromBottom (1));

    if (expanded && n == 0)
    {
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::body (13.0f));
        g.drawText ("No modifiers in this preset yet.", viewport.getBounds(), juce::Justification::centred);
    }
}

void ModifierRow::resized()
{
    auto r = getLocalBounds();
    auto bar = r.removeFromTop (barHeight).reduced (8, 4);
    showButton.setBounds (bar.removeFromRight (56));
    bar.removeFromRight (16);
    for (auto* b : { &addSteps, &addEnvelope, &addFollower, &addLfo })
    {
        b->setBounds (bar.removeFromRight (b == &addLfo ? 60 : 92));
        bar.removeFromRight (4);
    }

    viewport.setBounds (r.reduced (8, 4));
    int x = 0;
    for (auto* m : modules)
    {
        m->setBounds (x, 0, m->width(), moduleHeight);
        x += m->width() + 8;
    }
    strip.setSize (juce::jmax (x, viewport.getWidth()), moduleHeight);
}
