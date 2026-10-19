#pragma once

#include <JuceHeader.h>
#include <BinaryData.h>

//==============================================================================
/** PEQUOD style: dark charcoal, mustard yellow, Courier headings / Roboto body. */
namespace Pequod
{
    namespace Colours
    {
        const juce::Colour bg        { 0xff1c1c1c };
        const juce::Colour panel     { 0xff252525 };
        const juce::Colour panelHi   { 0xff2e2e2e };
        const juce::Colour well      { 0xff151515 };
        const juce::Colour rail      { 0xff101010 };
        const juce::Colour line      { 0xff3a3a3a };
        const juce::Colour mustard   { 0xffe0a82e };
        const juce::Colour mustardLo { 0xff7d6020 };
        const juce::Colour text      { 0xffede6d6 };
        const juce::Colour textDim   { 0xff8f8a80 };
        const juce::Colour danger    { 0xffc0503a };
    }

    /** Embedded fonts (Courier Prime for headings, Roboto for body) so the look is identical on every PC. */
    struct Typefaces
    {
        static juce::Typeface::Ptr load (const char* data, int size)
        {
            return juce::Typeface::createSystemTypefaceFor (data, (size_t) size);
        }

        static juce::Typeface::Ptr courier (bool bold)
        {
            static auto regular = load (BinaryData::CourierPrimeRegular_ttf, BinaryData::CourierPrimeRegular_ttfSize);
            static auto boldTf  = load (BinaryData::CourierPrimeBold_ttf,    BinaryData::CourierPrimeBold_ttfSize);
            return bold ? boldTf : regular;
        }

        static juce::Typeface::Ptr roboto (bool bold)
        {
            static auto regular = load (BinaryData::RobotoRegular_ttf, BinaryData::RobotoRegular_ttfSize);
            static auto boldTf  = load (BinaryData::RobotoBold_ttf,    BinaryData::RobotoBold_ttfSize);
            return bold ? boldTf : regular;
        }
    };

    inline juce::Font heading (float size, bool bold = true)
    {
        return juce::Font (juce::FontOptions (Typefaces::courier (bold)).withHeight (size));
    }

    inline juce::Font body (float size, bool bold = false)
    {
        return juce::Font (juce::FontOptions (Typefaces::roboto (bold)).withHeight (size));
    }

    /** Small screw head, as on a rack ear. */
    inline void drawScrew (juce::Graphics& g, juce::Point<float> c, float r = 3.5f)
    {
        g.setGradientFill (juce::ColourGradient (juce::Colour (0xff6a6a6a), c.x - r, c.y - r,
                                                 juce::Colour (0xff1e1e1e), c.x + r, c.y + r, false));
        g.fillEllipse (c.x - r, c.y - r, r * 2, r * 2);
        g.setColour (juce::Colour (0xff0c0c0c));
        g.drawLine (c.x - r * 0.6f, c.y, c.x + r * 0.6f, c.y, 1.2f);
    }

    /** Hardware-style module panel with rack ears. */
    inline void drawModulePanel (juce::Graphics& g, juce::Rectangle<float> r, bool active)
    {
        const float ear = 16.0f;
        g.setColour (Colours::rail);
        g.fillRoundedRectangle (r, 3.0f);

        auto face = r.reduced (ear, 0);
        g.setGradientFill (juce::ColourGradient (Colours::panelHi, face.getX(), face.getY(),
                                                 Colours::panel, face.getX(), face.getBottom(), false));
        g.fillRect (face);

        g.setColour (active ? Colours::mustard : Colours::mustardLo);
        g.fillRect (face.withHeight (2.0f));

        g.setColour (Colours::line);
        g.drawRect (face, 1.0f);

        for (float x : { r.getX() + ear * 0.5f, r.getRight() - ear * 0.5f })
        {
            drawScrew (g, { x, r.getY() + 10.0f });
            if (r.getHeight() > 40.0f)
                drawScrew (g, { x, r.getBottom() - 10.0f });
        }
    }
}

//==============================================================================
class PequodLookAndFeel : public juce::LookAndFeel_V4
{
public:
    PequodLookAndFeel()
    {
        auto scheme = getDarkColourScheme();
        scheme.setUIColour (ColourScheme::windowBackground, Pequod::Colours::bg);
        scheme.setUIColour (ColourScheme::widgetBackground, Pequod::Colours::panel);
        scheme.setUIColour (ColourScheme::menuBackground,   Pequod::Colours::panel);
        scheme.setUIColour (ColourScheme::outline,          Pequod::Colours::line);
        scheme.setUIColour (ColourScheme::defaultText,      Pequod::Colours::text);
        scheme.setUIColour (ColourScheme::defaultFill,      Pequod::Colours::mustardLo);
        scheme.setUIColour (ColourScheme::highlightedText,  Pequod::Colours::bg);
        scheme.setUIColour (ColourScheme::highlightedFill,  Pequod::Colours::mustard);
        scheme.setUIColour (ColourScheme::menuText,         Pequod::Colours::text);
        setColourScheme (scheme);

        setColour (juce::TextButton::buttonColourId,    Pequod::Colours::panelHi);
        setColour (juce::TextButton::buttonOnColourId,  Pequod::Colours::mustard);
        setColour (juce::TextButton::textColourOffId,   Pequod::Colours::text);
        setColour (juce::TextButton::textColourOnId,    Pequod::Colours::bg);
        setColour (juce::Slider::rotarySliderFillColourId,    Pequod::Colours::mustard);
        setColour (juce::Slider::rotarySliderOutlineColourId, Pequod::Colours::well);
        setColour (juce::Slider::thumbColourId,               Pequod::Colours::mustard);
        setColour (juce::Slider::trackColourId,               Pequod::Colours::mustardLo);
        setColour (juce::Slider::backgroundColourId,          Pequod::Colours::well);
        setColour (juce::Slider::textBoxTextColourId,         Pequod::Colours::text);
        setColour (juce::Slider::textBoxOutlineColourId,      juce::Colours::transparentBlack);
        setColour (juce::Label::textColourId,                 Pequod::Colours::text);
        setColour (juce::TextEditor::backgroundColourId,      Pequod::Colours::well);
        setColour (juce::TextEditor::outlineColourId,         Pequod::Colours::line);
        setColour (juce::TextEditor::focusedOutlineColourId,  Pequod::Colours::mustard);
        setColour (juce::TextEditor::textColourId,            Pequod::Colours::text);
        setColour (juce::TextEditor::highlightColourId,       Pequod::Colours::mustardLo);
        setColour (juce::CaretComponent::caretColourId,       Pequod::Colours::mustard);
        setColour (juce::ListBox::backgroundColourId,         Pequod::Colours::well);
        setColour (juce::ListBox::outlineColourId,            juce::Colours::transparentBlack);
        setColour (juce::ComboBox::backgroundColourId,        Pequod::Colours::well);
        setColour (juce::ComboBox::outlineColourId,           Pequod::Colours::line);
        setColour (juce::ComboBox::arrowColourId,             Pequod::Colours::mustard);
        setColour (juce::PopupMenu::backgroundColourId,       Pequod::Colours::panel);
        setColour (juce::PopupMenu::highlightedBackgroundColourId, Pequod::Colours::mustard);
        setColour (juce::PopupMenu::highlightedTextColourId,  Pequod::Colours::bg);
        setColour (juce::ScrollBar::thumbColourId,            Pequod::Colours::mustardLo);
        setColour (juce::ToggleButton::tickColourId,          Pequod::Colours::mustard);
        setColour (juce::ToggleButton::tickDisabledColourId,  Pequod::Colours::textDim);
        setColour (juce::ResizableWindow::backgroundColourId, Pequod::Colours::bg);
        setColour (juce::DocumentWindow::textColourId,        Pequod::Colours::text);
        setColour (juce::AlertWindow::backgroundColourId,     Pequod::Colours::panel);
        setColour (juce::AlertWindow::textColourId,           Pequod::Colours::text);
        setColour (juce::AlertWindow::outlineColourId,        Pequod::Colours::mustard);
        setColour (juce::TooltipWindow::backgroundColourId,   Pequod::Colours::well);
        setColour (juce::TooltipWindow::textColourId,         Pequod::Colours::text);
        setColour (juce::TooltipWindow::outlineColourId,      Pequod::Colours::mustardLo);

    }

    juce::Typeface::Ptr getTypefaceForFont (const juce::Font& f) override
    {
        // Everything not explicitly styled (dialogs, plugin list, generic editors) uses embedded Roboto.
        if (f.getTypefaceName() == juce::Font::getDefaultSansSerifFontName())
            return Pequod::Typefaces::roboto (f.isBold());
        return LookAndFeel_V4::getTypefaceForFont (f);
    }

    juce::Font getTextButtonFont (juce::TextButton&, int buttonHeight) override
    {
        return Pequod::heading (juce::jmin (14.0f, (float) buttonHeight * 0.55f));
    }

    juce::Font getLabelFont (juce::Label& l) override
    {
        return l.getFont();
    }

    void drawButtonBackground (juce::Graphics& g, juce::Button& b, const juce::Colour& bgColour,
                               bool highlighted, bool down) override
    {
        auto r = b.getLocalBounds().toFloat().reduced (0.5f);
        const auto onColour = b.findColour (juce::TextButton::buttonOnColourId);
        auto c = b.getToggleState() ? onColour : bgColour;
        if (down)             c = c.brighter (0.15f);
        else if (highlighted) c = c.brighter (0.07f);
        g.setColour (c);
        g.fillRoundedRectangle (r, 2.0f);
        g.setColour (b.getToggleState() ? onColour : Pequod::Colours::line);
        g.drawRoundedRectangle (r, 2.0f, 1.0f);
    }

    void drawRotarySlider (juce::Graphics& g, int x, int y, int w, int h, float pos,
                           float start, float end, juce::Slider& s) override
    {
        auto r = juce::Rectangle<float> ((float) x, (float) y, (float) w, (float) h).reduced (3.0f);
        auto size = juce::jmin (r.getWidth(), r.getHeight());
        auto c = r.getCentre();
        auto radius = size * 0.5f;
        auto angle = start + pos * (end - start);

        juce::Path track;
        track.addCentredArc (c.x, c.y, radius, radius, 0, start, end, true);
        g.setColour (Pequod::Colours::well);
        g.strokePath (track, juce::PathStrokeType (3.0f, juce::PathStrokeType::curved, juce::PathStrokeType::rounded));

        juce::Path arc;
        arc.addCentredArc (c.x, c.y, radius, radius, 0, start, angle, true);
        g.setColour (s.isEnabled() ? Pequod::Colours::mustard : Pequod::Colours::textDim);
        g.strokePath (arc, juce::PathStrokeType (3.0f, juce::PathStrokeType::curved, juce::PathStrokeType::rounded));

        auto knobR = radius - 5.0f;
        g.setGradientFill (juce::ColourGradient (juce::Colour (0xff3c3c3c), c.x, c.y - knobR,
                                                 juce::Colour (0xff141414), c.x, c.y + knobR, false));
        g.fillEllipse (c.x - knobR, c.y - knobR, knobR * 2, knobR * 2);
        g.setColour (Pequod::Colours::line);
        g.drawEllipse (c.x - knobR, c.y - knobR, knobR * 2, knobR * 2, 1.0f);

        juce::Path pointer;
        pointer.addRoundedRectangle (-1.25f, -knobR + 2.0f, 2.5f, knobR * 0.55f, 1.0f);
        g.setColour (Pequod::Colours::text);
        g.fillPath (pointer, juce::AffineTransform::rotation (angle).translated (c.x, c.y));
    }

    void drawLinearSlider (juce::Graphics& g, int x, int y, int w, int h, float pos, float, float,
                           juce::Slider::SliderStyle style, juce::Slider& s) override
    {
        if (style != juce::Slider::LinearHorizontal)
            return LookAndFeel_V4::drawLinearSlider (g, x, y, w, h, pos, 0, 0, style, s);

        auto r = juce::Rectangle<float> ((float) x, (float) y, (float) w, (float) h);
        auto track = r.withSizeKeepingCentre (r.getWidth(), 4.0f);
        g.setColour (Pequod::Colours::well);
        g.fillRoundedRectangle (track, 2.0f);
        g.setColour (Pequod::Colours::line);
        g.drawLine (r.getCentreX(), track.getY() - 4, r.getCentreX(), track.getBottom() + 4, 1.0f);

        auto thumb = juce::Rectangle<float> (10.0f, juce::jmin (18.0f, r.getHeight())).withCentre ({ pos, r.getCentreY() });
        g.setColour (Pequod::Colours::mustard);
        g.fillRoundedRectangle (thumb, 2.0f);
        g.setColour (Pequod::Colours::bg);
        g.drawLine (thumb.getCentreX(), thumb.getY() + 3, thumb.getCentreX(), thumb.getBottom() - 3, 1.0f);
    }

    void drawPopupMenuBackground (juce::Graphics& g, int w, int h) override
    {
        g.fillAll (Pequod::Colours::panel);
        g.setColour (Pequod::Colours::mustardLo);
        g.drawRect (0, 0, w, h);
    }

    juce::Font getPopupMenuFont() override { return Pequod::body (14.0f); }

    void drawScrollbar (juce::Graphics& g, juce::ScrollBar&, int x, int y, int w, int h, bool vertical,
                        int thumbStart, int thumbSize, bool over, bool) override
    {
        auto thumb = vertical ? juce::Rectangle<int> (x, thumbStart, w, thumbSize)
                              : juce::Rectangle<int> (thumbStart, y, thumbSize, h);
        g.setColour (over ? Pequod::Colours::mustard : Pequod::Colours::mustardLo);
        g.fillRoundedRectangle (thumb.reduced (3).toFloat(), 2.0f);
    }
};

//==============================================================================
/** Round power LED button used on modules. */
class PowerButton : public juce::Button
{
public:
    PowerButton() : Button ("power") { setClickingTogglesState (true); }

    void paintButton (juce::Graphics& g, bool over, bool) override
    {
        auto r = getLocalBounds().toFloat().withSizeKeepingCentre (14.0f, 14.0f);
        if (getToggleState())
        {
            g.setColour (Pequod::Colours::mustard.withAlpha (0.25f));
            g.fillEllipse (r.expanded (3.0f));
            g.setColour (Pequod::Colours::mustard);
        }
        else
        {
            g.setColour (over ? juce::Colour (0xff4a4a4a) : juce::Colour (0xff333333));
        }
        g.fillEllipse (r);
        g.setColour (Pequod::Colours::rail);
        g.drawEllipse (r, 1.5f);
    }
};

//==============================================================================
/** Stereo peak meter reading a pair of atomics. */
class PeakMeter : public juce::Component
{
public:
    void setLevels (float l, float r)
    {
        auto toPos = [] (float v) { return juce::jlimit (0.0f, 1.0f, (juce::Decibels::gainToDecibels (v, -60.0f) + 60.0f) / 66.0f); };
        auto nl = toPos (l), nr = toPos (r);
        if (std::abs (nl - levelL) > 0.002f || std::abs (nr - levelR) > 0.002f)
        {
            levelL = nl; levelR = nr;
            repaint();
        }
    }

    void paint (juce::Graphics& g) override
    {
        auto r = getLocalBounds().toFloat();
        auto barW = (r.getWidth() - 2.0f) * 0.5f;
        for (int ch = 0; ch < 2; ++ch)
        {
            auto bar = juce::Rectangle<float> (r.getX() + (float) ch * (barW + 2.0f), r.getY(), barW, r.getHeight());
            g.setColour (Pequod::Colours::well);
            g.fillRect (bar);
            auto lvl = ch == 0 ? levelL : levelR;
            auto filled = bar.withTop (bar.getBottom() - bar.getHeight() * lvl);
            g.setColour (lvl > 0.9f ? Pequod::Colours::danger : Pequod::Colours::mustard);
            g.fillRect (filled);
        }
    }

private:
    float levelL = 0, levelR = 0;
};

//==============================================================================
/** Slider whose right-click offers "MIDI / OSC learn". */
class LearnableSlider : public juce::Slider
{
public:
    std::function<void()> onLearn;

    void mouseDown (const juce::MouseEvent& e) override
    {
        if (e.mods.isPopupMenu() && onLearn)
        {
            juce::PopupMenu m;
            m.addItem ("MIDI / OSC learn", onLearn);
            m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (this));
            return;
        }
        Slider::mouseDown (e);
    }
};

//==============================================================================
/** TextButton whose right-click offers "MIDI / OSC learn". */
class LearnableButton : public juce::TextButton
{
public:
    using juce::TextButton::TextButton;
    std::function<void()> onLearn;

    void mouseDown (const juce::MouseEvent& e) override
    {
        if (e.mods.isPopupMenu() && onLearn)
        {
            juce::PopupMenu m;
            m.addItem ("MIDI / OSC learn", onLearn);
            m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (this));
            return;
        }
        TextButton::mouseDown (e);
    }
};
