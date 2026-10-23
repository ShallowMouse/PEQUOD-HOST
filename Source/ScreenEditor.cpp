#include "ScreenEditor.h"

namespace
{
    void styleSmall (juce::TextButton& b)
    {
        b.setColour (juce::TextButton::buttonColourId, Pequod::Colours::well);
        b.setColour (juce::TextButton::buttonOnColourId, Pequod::Colours::mustard);
        b.setColour (juce::TextButton::textColourOnId, Pequod::Colours::rail);
    }

    void styleLabel (juce::Label& l, const juce::String& t)
    {
        l.setText (t, juce::dontSendNotification);
        l.setFont (Pequod::heading (11.0f));
        l.setColour (juce::Label::textColourId, Pequod::Colours::textDim);
    }

    void styleNumber (juce::Slider& s, int lo, int hi)
    {
        s.setSliderStyle (juce::Slider::IncDecButtons);
        s.setTextBoxStyle (juce::Slider::TextBoxLeft, false, 44, 24);
        s.setRange (lo, hi, 1);
        s.setIncDecButtonsMode (juce::Slider::incDecButtonsDraggable_Vertical);
        s.setColour (juce::Slider::textBoxBackgroundColourId, Pequod::Colours::well);
        s.setColour (juce::Slider::textBoxOutlineColourId, Pequod::Colours::line);
    }

    juce::String describe (const ScreenLayout::Element& e)
    {
        using K = ScreenLayout::Element::Kind;
        if (e.kind == K::box)   return e.h <= 1 || e.w <= 1 ? "LINE" : "FILLED BOX";
        if (e.kind == K::frame) return "BOX";
        return e.text.isEmpty() ? juce::String ("(empty text)") : e.text;
    }
}

//==============================================================================
/** The screen at a whole-number scale: click to pick, drag to move. */
class ScreenEditor::Preview : public juce::Component
{
public:
    explicit Preview (ScreenEditor& o) : owner (o) { setWantsKeyboardFocus (true); }

    int scale() const
    {
        return juce::jmax (1, juce::jmin ((getWidth() - 8) / ScreenLayout::width, (getHeight() - 8) / ScreenLayout::height));
    }
    juce::Rectangle<int> screenArea() const
    {
        const int s = scale();
        return juce::Rectangle<int> (ScreenLayout::width * s, ScreenLayout::height * s).withCentre (getLocalBounds().getCentre());
    }

    void paint (juce::Graphics& g) override
    {
        auto area = screenArea();
        const int s = scale();

        // the controller's screen bezel
        g.setColour (juce::Colour (0xff0a0a0a));
        g.fillRoundedRectangle (area.expanded (4).toFloat(), 4.0f);

        items = owner.mgr.resolveScreen (owner.layout, true);
        auto img = ScreenLayout::render (items);
        // the OLED's pixels are pale blue-white on black
        g.setImageResamplingQuality (juce::Graphics::lowResamplingQuality);
        g.drawImage (img, area.toFloat());
        g.setColour (juce::Colour (0xff9fd8ff).withAlpha (0.10f));
        g.fillRect (area);

        // faint pixel grid when it's big enough
        if (s >= 4)
        {
            g.setColour (juce::Colours::black.withAlpha (0.35f));
            for (int x = 1; x < ScreenLayout::width; ++x)  g.fillRect (area.getX() + x * s, area.getY(), 1, area.getHeight());
            for (int y = 1; y < ScreenLayout::height; ++y) g.fillRect (area.getX(), area.getY() + y * s, area.getWidth(), 1);
        }

        // selection
        for (auto& it : items)
            if (it.element == owner.selected)
            {
                auto b = it.bounds();
                auto r = juce::Rectangle<int> (area.getX() + b.getX() * s, area.getY() + b.getY() * s, b.getWidth() * s, b.getHeight() * s);
                g.setColour (Pequod::Colours::mustard);
                g.drawRect (r.expanded (2), 2);
            }
    }

    int hit (juce::Point<int> p) const
    {
        auto area = screenArea();
        const int s = scale();
        const juce::Point<int> px ((p.x - area.getX()) / s, (p.y - area.getY()) / s);
        // topmost first; thin lines get a little extra room
        for (auto it = items.rbegin(); it != items.rend(); ++it)
            if (it->bounds().expanded (it->kind == 't' ? 0 : 2).contains (px))
                return it->element;
        return -1;
    }

    void mouseDown (const juce::MouseEvent& e) override
    {
        grabKeyboardFocus();
        owner.select (hit (e.getPosition()));
        if (auto* el = owner.current())
        {
            dragStart = { el->x, el->y };
            dragging = true;
        }
    }

    void mouseDrag (const juce::MouseEvent& e) override
    {
        auto* el = owner.current();
        if (! dragging || el == nullptr) return;
        const int s = scale();
        el->x = juce::jlimit (-64, 192, dragStart.x + juce::roundToInt ((float) e.getDistanceFromDragStartX() / (float) s));
        el->y = juce::jlimit (-32, 96, dragStart.y + juce::roundToInt ((float) e.getDistanceFromDragStartY() / (float) s));
        owner.updateProperties();
        owner.changed (false);           // preview only while dragging
    }

    void mouseUp (const juce::MouseEvent&) override
    {
        if (dragging) owner.changed (true);
        dragging = false;
    }

    bool keyPressed (const juce::KeyPress& k) override { return owner.keyPressed (k); }

private:
    ScreenEditor& owner;
    std::vector<ScreenLayout::Item> items;
    juce::Point<int> dragStart;
    bool dragging = false;
};

//==============================================================================
ScreenEditor::ScreenEditor (ControllerManager& m) : mgr (m)
{
    preview = std::make_unique<Preview> (*this);
    addAndMakeVisible (*preview);

    notice.setFont (Pequod::body (13.0f));
    notice.setColour (juce::Label::textColourId, Pequod::Colours::mustard);
    notice.setJustificationType (juce::Justification::centredLeft);
    addChildComponent (notice);

    useToggle.setButtonText ("Use this layout on the controller (off = the standard screen)");
    useToggle.onClick = [this] { custom = useToggle.getToggleState(); changed(); };
    barToggle.setButtonText ("White message bar when a switch is pressed (off: messages only show where you put {msg})");
    barToggle.onClick = [this] { layout.messageBar = barToggle.getToggleState(); changed(); };
    for (auto* t : { &useToggle, &barToggle, &invertToggle, &fitToggle })
    {
        t->setColour (juce::ToggleButton::tickColourId, Pequod::Colours::mustard);
        t->setColour (juce::ToggleButton::textColourId, Pequod::Colours::text);
        addAndMakeVisible (t);
    }

    for (auto* b : { &addText, &addLine, &addBox, &removeButton, &upButton, &downButton, &standardButton, &insertButton })
    {
        styleSmall (*b);
        addAndMakeVisible (b);
    }
    addText.onClick = [this] { add (ScreenLayout::Element::Kind::text); };
    addLine.onClick = [this] { add (ScreenLayout::Element::Kind::box); };
    addBox.onClick  = [this] { add (ScreenLayout::Element::Kind::frame); };
    removeButton.onClick = [this]
    {
        if (current() == nullptr) return;
        layout.elements.erase (layout.elements.begin() + selected);
        selected = juce::jmin (selected, (int) layout.elements.size() - 1);
        rebuildRows();
        changed();
    };
    upButton.onClick   = [this] { moveSelected (-1); };
    downButton.onClick = [this] { moveSelected (1); };
    standardButton.onClick = [this]
    {
        layout = ScreenLayout::Layout::standard();
        selected = -1;
        rebuildRows();
        changed();
    };
    insertButton.onClick = [this] { showTokenMenu(); };
    insertButton.setTooltip ("Add something PEQUOD fills in: preset name, BPM, effect on/off...");

    styleLabel (textLabel, "TEXT");
    styleLabel (sizeLabel, "SIZE");
    styleLabel (alignLabel, "LINE UP");
    styleLabel (posLabel, "X / Y");
    styleLabel (boxLabel, "WIDTH / HEIGHT");
    for (auto* l : { &textLabel, &sizeLabel, &alignLabel, &posLabel, &boxLabel }) addAndMakeVisible (l);

    textEditor.setFont (Pequod::heading (14.0f, false));
    textEditor.setColour (juce::TextEditor::backgroundColourId, Pequod::Colours::well);
    textEditor.setColour (juce::TextEditor::outlineColourId, Pequod::Colours::line);
    textEditor.setColour (juce::TextEditor::textColourId, Pequod::Colours::text);
    textEditor.onTextChange = [this]
    {
        if (auto* e = current()) { e->text = textEditor.getText(); if (auto* r = rows[selected]) r->setButtonText (describe (*e)); changed (false); startTimer (400); }
    };
    addAndMakeVisible (textEditor);

    for (int i = 1; i <= 4; ++i)
    {
        auto* b = sizeButtons.add (new juce::TextButton (juce::String (i)));
        styleSmall (*b);
        b->setClickingTogglesState (false);
        b->onClick = [this, i] { if (auto* e = current()) { e->size = i; updateProperties(); changed(); } };
        b->setTooltip (juce::String (i * 6) + " x " + juce::String (i * 8) + " pixels per letter");
        addAndMakeVisible (b);
    }
    const char* aligns[] = { "LEFT", "CENTRE", "RIGHT" };
    for (int i = 0; i < 3; ++i)
    {
        auto* b = alignButtons.add (new juce::TextButton (aligns[i]));
        styleSmall (*b);
        b->onClick = [this, i] { if (auto* e = current()) { e->align = i; updateProperties(); changed(); } };
        addAndMakeVisible (b);
    }
    invertToggle.setButtonText ("Black on white");
    invertToggle.onClick = [this] { if (auto* e = current()) { e->inverted = invertToggle.getToggleState(); changed(); } };
    fitToggle.setButtonText ("Shrink long text to fit");
    fitToggle.onClick = [this] { if (auto* e = current()) { e->fit = fitToggle.getToggleState(); changed(); } };

    styleNumber (xSlider, -64, 192);
    styleNumber (ySlider, -32, 96);
    styleNumber (wSlider, 1, 128);
    styleNumber (hSlider, 1, 64);
    xSlider.onValueChange = [this] { if (auto* e = current()) { e->x = (int) xSlider.getValue(); changed(); } };
    ySlider.onValueChange = [this] { if (auto* e = current()) { e->y = (int) ySlider.getValue(); changed(); } };
    wSlider.onValueChange = [this] { if (auto* e = current()) { e->w = (int) wSlider.getValue(); changed(); } };
    hSlider.onValueChange = [this] { if (auto* e = current()) { e->h = (int) hSlider.getValue(); changed(); } };
    for (auto* s : { &xSlider, &ySlider, &wSlider, &hSlider }) addAndMakeVisible (s);

    setWantsKeyboardFocus (true);
    reload();
}

ScreenEditor::~ScreenEditor() { stopTimer(); }

void ScreenEditor::reload()
{
    layout = mgr.getScreenLayout();
    custom = mgr.hasCustomScreen();
    selected = juce::jmin (selected, (int) layout.elements.size() - 1);
    useToggle.setToggleState (custom, juce::dontSendNotification);
    barToggle.setToggleState (layout.messageBar, juce::dontSendNotification);

    const bool ok = mgr.supportsScreenLayouts();
    notice.setVisible (mgr.isConnected() && ! ok);
    notice.setText ("This controller's firmware (" + mgr.getInfo().fw + ") shows the standard screen. Update its firmware "
                    "(SETTINGS) to use your own layout. You can make one now.", juce::dontSendNotification);
    rebuildRows();
    resized();
}

ScreenLayout::Element* ScreenEditor::current()
{
    return juce::isPositiveAndBelow (selected, (int) layout.elements.size()) ? &layout.elements[(size_t) selected] : nullptr;
}

void ScreenEditor::select (int index)
{
    selected = index;
    for (int i = 0; i < rows.size(); ++i) rows[i]->setToggleState (i == selected, juce::dontSendNotification);
    updateProperties();
    resized();
    preview->repaint();
}

void ScreenEditor::rebuildRows()
{
    rows.clear();
    for (size_t i = 0; i < layout.elements.size(); ++i)
    {
        auto* b = rows.add (new juce::TextButton (describe (layout.elements[i])));
        styleSmall (*b);
        b->onClick = [this, i] { select ((int) i); };
        addAndMakeVisible (b);
    }
    select (selected);
    resized();
    if (onSizeChanged) onSizeChanged();
}

void ScreenEditor::refreshPreview()
{
    preview->repaint();
}

void ScreenEditor::updateProperties()
{
    auto* e = current();
    using K = ScreenLayout::Element::Kind;
    const bool isText = e != nullptr && e->kind == K::text;
    const bool isShape = e != nullptr && e->kind != K::text;

    for (auto* c : std::initializer_list<juce::Component*> { &textLabel, &textEditor, &insertButton, &sizeLabel, &alignLabel, &invertToggle, &fitToggle })
        c->setVisible (isText);
    for (auto* b : sizeButtons)  b->setVisible (isText);
    for (auto* b : alignButtons) b->setVisible (isText);
    for (auto* c : std::initializer_list<juce::Component*> { &boxLabel, &wSlider, &hSlider }) c->setVisible (isShape);
    for (auto* c : std::initializer_list<juce::Component*> { &posLabel, &xSlider, &ySlider, &removeButton, &upButton, &downButton })
        c->setVisible (e != nullptr);

    if (e == nullptr) { repaint(); return; }

    if (isText)
    {
        if (textEditor.getText() != e->text) textEditor.setText (e->text, juce::dontSendNotification);
        for (int i = 0; i < sizeButtons.size(); ++i)  sizeButtons[i]->setToggleState (e->size == i + 1, juce::dontSendNotification);
        for (int i = 0; i < alignButtons.size(); ++i) alignButtons[i]->setToggleState (e->align == i, juce::dontSendNotification);
        invertToggle.setToggleState (e->inverted, juce::dontSendNotification);
        fitToggle.setToggleState (e->fit, juce::dontSendNotification);
        posLabel.setText (e->align == 1 ? "CENTRE X / TOP Y" : e->align == 2 ? "RIGHT EDGE X / TOP Y" : "LEFT X / TOP Y", juce::dontSendNotification);
    }
    else
    {
        wSlider.setValue (e->w, juce::dontSendNotification);
        hSlider.setValue (e->h, juce::dontSendNotification);
        posLabel.setText ("LEFT X / TOP Y", juce::dontSendNotification);
    }
    xSlider.setValue (e->x, juce::dontSendNotification);
    ySlider.setValue (e->y, juce::dontSendNotification);
    repaint();
}

void ScreenEditor::changed (bool sendNow)
{
    preview->repaint();
    if (sendNow)
    {
        stopTimer();
        mgr.setScreenLayout (layout, custom);
    }
}

void ScreenEditor::timerCallback()
{
    // typing: send once it pauses
    stopTimer();
    mgr.setScreenLayout (layout, custom);
}

void ScreenEditor::add (ScreenLayout::Element::Kind kind)
{
    ScreenLayout::Element e;
    e.kind = kind;
    if (kind == ScreenLayout::Element::Kind::text) { e.text = "TEXT"; e.x = 64; e.y = 28; e.size = 1; e.align = 1; }
    else if (kind == ScreenLayout::Element::Kind::box) { e.x = 0; e.y = 32; e.w = 128; e.h = 1; }
    else { e.x = 32; e.y = 16; e.w = 64; e.h = 32; }
    layout.elements.push_back (e);
    if (! custom) { custom = true; useToggle.setToggleState (true, juce::dontSendNotification); }
    selected = (int) layout.elements.size() - 1;
    rebuildRows();
    changed();
    if (kind == ScreenLayout::Element::Kind::text) { textEditor.grabKeyboardFocus(); textEditor.selectAll(); }
}

void ScreenEditor::moveSelected (int delta)
{
    const int to = selected + delta;
    if (current() == nullptr || ! juce::isPositiveAndBelow (to, (int) layout.elements.size())) return;
    std::swap (layout.elements[(size_t) selected], layout.elements[(size_t) to]);
    selected = to;
    rebuildRows();
    changed();
}

void ScreenEditor::nudge (int dx, int dy)
{
    if (auto* e = current())
    {
        e->x += dx;
        e->y += dy;
        updateProperties();
        changed();
    }
}

bool ScreenEditor::keyPressed (const juce::KeyPress& k)
{
    if (textEditor.hasKeyboardFocus (false)) return false;
    const int step = k.getModifiers().isShiftDown() ? 8 : 1;
    if (k.isKeyCode (juce::KeyPress::leftKey))  { nudge (-step, 0); return true; }
    if (k.isKeyCode (juce::KeyPress::rightKey)) { nudge (step, 0);  return true; }
    if (k.isKeyCode (juce::KeyPress::upKey))    { nudge (0, -step); return true; }
    if (k.isKeyCode (juce::KeyPress::downKey))  { nudge (0, step);  return true; }
    if ((k.isKeyCode (juce::KeyPress::deleteKey) || k.isKeyCode (juce::KeyPress::backspaceKey)) && current() != nullptr)
    {
        removeButton.triggerClick();
        return true;
    }
    return false;
}

void ScreenEditor::showTokenMenu()
{
    juce::PopupMenu m;
    m.addSectionHeader ("PEQUOD FILLS THESE IN");
    for (auto& t : ScreenLayout::tokens())
    {
        const juce::String token (t.text);
        m.addItem (token + "   " + t.description, [this, token]
        {
            textEditor.grabKeyboardFocus();
            textEditor.insertTextAtCaret (token);
        });
    }
    m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (&insertButton));
}

//==============================================================================
void ScreenEditor::paint (juce::Graphics& g)
{
    g.setColour (Pequod::Colours::textDim);
    g.setFont (Pequod::heading (11.0f));
    if (! listArea.isEmpty())  g.drawText ("ON THE SCREEN (TOP OF THE LIST IS DRAWN FIRST)", listArea.withHeight (16), juce::Justification::centredLeft);
    if (! propsArea.isEmpty())
    {
        g.drawText (current() != nullptr ? juce::String ("SELECTED") : juce::String ("CLICK SOMETHING ON THE SCREEN TO CHANGE IT"),
                    propsArea.withHeight (16), juce::Justification::centredLeft);
        if (current() != nullptr)
        {
            g.setFont (Pequod::body (12.0f));
            g.drawText ("Drag it on the screen, or use the arrow keys (Shift = 8 pixels).",
                        propsArea.withTrimmedTop (16).withHeight (16), juce::Justification::centredLeft);
        }
    }
}

int ScreenEditor::getIdealHeight (int width) const
{
    const int previewH = juce::jlimit (200, 420, (width - 40) / 2 + 16);
    return 16 + (notice.isVisible() ? 30 : 0) + previewH + 12 + 60 + 12 + juce::jmax (36 + 34 * (int) layout.elements.size() + 40, 420);
}

void ScreenEditor::resized()
{
    auto r = getLocalBounds().reduced (16, 12);
    if (notice.isVisible()) notice.setBounds (r.removeFromTop (26)), r.removeFromTop (4);

    const int previewH = juce::jlimit (200, 420, (r.getWidth() - 8) / 2 + 16);
    previewArea = r.removeFromTop (previewH);
    preview->setBounds (previewArea);
    r.removeFromTop (12);

    useToggle.setBounds (r.removeFromTop (26));
    barToggle.setBounds (r.removeFromTop (26));
    r.removeFromTop (12);

    const bool twoCols = r.getWidth() > 700;
    listArea = twoCols ? r.removeFromLeft (r.getWidth() / 2 - 8) : r.removeFromTop (36 + 34 * (int) layout.elements.size() + 40);
    if (twoCols) r.removeFromLeft (16);
    propsArea = r;

    // list
    auto l = listArea.withTrimmedTop (20);
    auto addRow = l.removeFromTop (30);
    for (auto* b : { &addText, &addLine, &addBox })
    {
        b->setBounds (addRow.removeFromLeft (80));
        addRow.removeFromLeft (6);
    }
    l.removeFromTop (8);
    for (auto* b : rows)
    {
        b->setBounds (l.removeFromTop (30));
        l.removeFromTop (4);
    }
    l.removeFromTop (8);
    standardButton.setBounds (l.removeFromTop (30).withWidth (juce::jmin (l.getWidth(), 320)));

    // properties
    auto p = propsArea.withTrimmedTop (38);
    auto line = [&p] (int h) { auto row = p.removeFromTop (h); p.removeFromTop (8); return row; };

    if (auto* e = current())
    {
        if (e->kind == ScreenLayout::Element::Kind::text)
        {
            textLabel.setBounds (p.removeFromTop (16));
            auto t = line (30);
            insertButton.setBounds (t.removeFromRight (96));
            t.removeFromRight (6);
            textEditor.setBounds (t);

            sizeLabel.setBounds (p.removeFromTop (16));
            auto sz = line (30);
            for (auto* b : sizeButtons) { b->setBounds (sz.removeFromLeft (40)); sz.removeFromLeft (4); }

            alignLabel.setBounds (p.removeFromTop (16));
            auto al = line (30);
            for (auto* b : alignButtons) { b->setBounds (al.removeFromLeft (80)); al.removeFromLeft (4); }

            auto tg = line (26);
            invertToggle.setBounds (tg.removeFromLeft (170));
            fitToggle.setBounds (tg);
        }
        else
        {
            boxLabel.setBounds (p.removeFromTop (16));
            auto bx = line (28);
            wSlider.setBounds (bx.removeFromLeft (130));
            bx.removeFromLeft (8);
            hSlider.setBounds (bx.removeFromLeft (130));
        }

        posLabel.setBounds (p.removeFromTop (16));
        auto pos = line (28);
        xSlider.setBounds (pos.removeFromLeft (130));
        pos.removeFromLeft (8);
        ySlider.setBounds (pos.removeFromLeft (130));

        auto ord = line (30);
        upButton.setBounds (ord.removeFromLeft (60));
        ord.removeFromLeft (4);
        downButton.setBounds (ord.removeFromLeft (70));
        ord.removeFromLeft (12);
        removeButton.setBounds (ord.removeFromLeft (80));
    }
}
