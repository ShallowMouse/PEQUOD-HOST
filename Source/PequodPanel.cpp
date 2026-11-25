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
    const Colour CREAM       { 0xfff6eedc };

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
    /** (checking: every piece of text a panel draws, where it lands - see PEQUOD_PANEL_SHOTS) */
    std::vector<std::pair<juce::Rectangle<float>, juce::String>>* textLog = nullptr;
    /** The panel being painted: text is nudged back inside it rather than cut off at an edge. */
    juce::Rectangle<float> textBounds;

    void say (juce::Graphics& g, const juce::String& s, Pt at, int ax, int ay, float size, Colour c, bool mono = false)
    {
        auto fnt = font (size, mono);
        g.setFont (fnt);
        g.setColour (c);
        const float w = juce::GlyphArrangement::getStringWidth (fnt, s) + 2.0f;
        const float h = fnt.getHeight();
        float x = ax < 0 ? at.x : ax > 0 ? at.x - w : at.x - w * 0.5f;
        const float y = ay < 0 ? at.y : ay > 0 ? at.y - h : at.y - h * 0.5f;
        if (! textBounds.isEmpty() && w < textBounds.getWidth() - 8.0f)
            x = juce::jlimit (textBounds.getX() + 4.0f, textBounds.getRight() - w - 2.0f, x);
        if (textLog != nullptr && s.trim().isNotEmpty())
            textLog->push_back ({ Rect (x, y, w - 2.0f, h * 0.8f).translated (0.0f, h * 0.1f), s });
        g.drawText (s, Rect (x, y, w, h), ax < 0 ? juce::Justification::centredLeft : ax > 0 ? juce::Justification::centredRight : juce::Justification::centred, false);
    }

    /** say(), shrunk until it fits `maxW` (the rack faceplate's name block). */
    void sayFit (juce::Graphics& g, const juce::String& s, Pt at, float maxW, float size, Colour c)
    {
        float sz = size;
        while (sz > 7.0f && juce::GlyphArrangement::getStringWidth (font (sz), s) > maxW) sz -= 0.5f;
        auto t = s;
        while (t.length() > 4 && juce::GlyphArrangement::getStringWidth (font (sz), t) > maxW)
            t = t.dropLastCharacters (2).trimEnd() + juce::String (juce::CharPointer_UTF8 ("\xe2\x80\xa6"));
        say (g, t, at, -1, 0, sz, c);
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

    /** A screw slot's turn: random-looking, but the same every time that screw is drawn. */
    float slotTurn (Pt c, float seed)
    {
        auto h = (uint32_t) (juce::roundToInt (c.x * 3.0f) * 73856093) ^ (uint32_t) (juce::roundToInt (c.y * 3.0f) * 19349663)
               ^ (uint32_t) (juce::roundToInt (seed * 1000.0f) * 83492791);
        h ^= h >> 16; h *= 0x7feb352dU; h ^= h >> 15; h *= 0x846ca68bU; h ^= h >> 16;
        return (float) (h & 0xffff) / 65536.0f * pi;
    }
}

//==============================================================================
/*  PEQUOD Hardware's faceplates: each unit in the colours and knobs of the box it emulates
    (its paint, its ink, the knobs it wore), with no maker's name or logo on it. */
void PequodPanel::applyHardwareFace (const juce::String& name)
{
    struct F { const char* names; uint32_t plate, edge, ink, inkSoft, block, blockInk; KnobStyle knob; uint32_t cap; std::vector<uint32_t> caps; uint32_t meter; bool brushed; };
    static const std::vector<F> faces {
        // compressors
        { "Fairchild 670",        0xff5d625f, 0xff3d413f, 0xfff0ece0, 0xffb8b4a8, 0xff3a3e3c, 0xfff0ece0, KnobStyle::skirted,   0xff151515, {}, 0xfff3e7c2, false },
        { "LA-2A",                0xffb9bcbd, 0xff8a8d8e, 0xff1a1a1a, 0xff4a4a4a, 0xff2a2c2d, 0xffe8e8e6, KnobStyle::skirted,   0xff111111, {}, 0xfff3e7c2, true },
        { "LA-3A",                0xff1d1e20, 0xff3a3b3e, 0xffd8d8d4, 0xff8c8c88, 0xffb9bcbd, 0xff111111, KnobStyle::skirted,   0xff111111, {}, 0xfff3e7c2, false },
        { "dbx 160",              0xffc4c6c7, 0xff8f9192, 0xff151515, 0xff4b4b4b, 0xff1a1a1a, 0xffe0e0dc, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, true },
        { "SSL Bus Compressor",   0xff6d757b, 0xff4b5156, 0xffe9ebec, 0xffb4babe, 0xff40464b, 0xffe9ebec, KnobStyle::capped,    0xff2a2a2a, { 0xffc33a32, 0xff2f5fae, 0xff3b8a4a, 0xffe0e0dc }, 0xff141414, false },
        { "API 2500",             0xff2b2d30, 0xff45484c, 0xffe6e6e2, 0xff9da0a3, 0xff5a7fa6, 0xfff3f3f0, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, false },
        { "Neve 33609",           0xff6f7f8b, 0xff4d5962, 0xfff1f1ee, 0xffc4cbd0, 0xff4a5660, 0xfff1f1ee, KnobStyle::capped,    0xff6e7175, { 0xffb2443a, 0xff8a8e92 }, 0xfff3e7c2, false },
        { "Distressor",           0xff141516, 0xff2e3032, 0xffe0e2e4, 0xff8e9296, 0xff2a2c2e, 0xffe0e2e4, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, false },
        // equalisers and preamps
        { "Pultec EQP-1A|Pultec MEQ-5", 0xff6b7d8c, 0xff4a5865, 0xfff3f2ec, 0xffc9d0d6, 0xff4a5865, 0xfff3f2ec, KnobStyle::chicken, 0xff151515, {}, 0xfff3e7c2, false },
        { "Neve 1073",            0xff5f7486, 0xff43535f, 0xfff1f1ee, 0xffc4ccd2, 0xff3c4a55, 0xfff1f1ee, KnobStyle::capped,    0xff6e7175, { 0xffb2443a, 0xff3e6aa8, 0xff8a8e92 }, 0xfff3e7c2, false },
        { "API 550A|API 312",     0xff8e9195, 0xff686b6e, 0xff151515, 0xff3e4144, 0xff1e2022, 0xffe6e6e2, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, true },
        { "Lang PEQ-1",           0xff9a9a92, 0xff707068, 0xff151515, 0xff444440, 0xff2a2a28, 0xffeeeee8, KnobStyle::chicken,   0xff151515, {}, 0xfff3e7c2, true },
        // reverbs and effects
        { "Lexicon 224",          0xffd9cfb8, 0xffa99f88, 0xff3b2f24, 0xff786a58, 0xff8a6a46, 0xfff6eedc, KnobStyle::aluminium, 0xff2a2a2a, {}, 0xff141414, false },
        { "Lexicon 480L|PCM70",   0xff1e1f21, 0xff3a3b3e, 0xffd8d8d4, 0xff8c8c88, 0xff6a2626, 0xffeeeeea, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, false },
        { "EMT 250",              0xff3b4a5a, 0xff2a3642, 0xffe8ecef, 0xffaab5bf, 0xffc7a24a, 0xff1a1a1a, KnobStyle::capped,    0xff2a2a2a, { 0xffc7a24a, 0xffe8ecef }, 0xff141414, false },
        { "AMS RMX16",            0xff1f2d4a, 0xff314366, 0xffe6eaf2, 0xff9aa6bd, 0xff121b2e, 0xffe6eaf2, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, false },
        { "Yamaha REV7|Yamaha SPX90", 0xff27292c, 0xff45484c, 0xffe8e8e4, 0xff9a9d9f, 0xff3d4045, 0xffe8e8e4, KnobStyle::capped,  0xff2a2a2a, { 0xff8e9196 }, 0xff141414, false },
        { "H3000",                0xff121314, 0xff2e3032, 0xffdcdde0, 0xff8a8d90, 0xff2a2c2e, 0xffdcdde0, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, false },
        { "Echoplex EP-3",        0xffb8b9b6, 0xff858683, 0xff161616, 0xff474747, 0xff2a2624, 0xffe8e4dc, KnobStyle::chicken,   0xff151515, {}, 0xfff3e7c2, true },
        { "Aural Exciter",        0xff17181a, 0xff34363a, 0xffd9b86a, 0xff9b8550, 0xffd9b86a, 0xff17181a, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, false },
        { "Sonic Maximizer",      0xff1b1c1e, 0xff383a3e, 0xffe2e2de, 0xff96969a, 0xff2a2c2e, 0xffe2e2de, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, false },
        { "Flanger/Doubler",      0xff151617, 0xff323436, 0xffe4e4e0, 0xff94948f, 0xffb83a2c, 0xfff2f2ee, KnobStyle::capped,    0xff2a2a2a, { 0xffb83a2c, 0xff2f5fae, 0xffd9a531 }, 0xff141414, false },
        // synthesizers
        { "ARP Odyssey",          0xff1a1a1a, 0xff3a3a3a, 0xffeeeeea, 0xff9a9a96, 0xffd86a22, 0xff1a1a1a, KnobStyle::capped,    0xff2a2a2a, { 0xffeeeeea }, 0xff141414, false },
        { "ARP 2600",             0xff4b5e73, 0xff33424f, 0xffeef0f2, 0xffb0bcc8, 0xff2c3946, 0xffeef0f2, KnobStyle::capped,    0xff2a2a2a, { 0xffeeeeea, 0xffd86a22, 0xff2f5fae }, 0xff141414, false },
        { "Oberheim OB-X",        0xff17181a, 0xff34363a, 0xffe8e8e4, 0xff9a9a96, 0xff6e4a2e, 0xffe8e8e4, KnobStyle::aluminium, 0xff1a1a1a, {}, 0xff141414, false },
        { "Roland Jupiter-8",     0xff1c1d1f, 0xff3a3b3e, 0xffe8e8e4, 0xff9a9a96, 0xff2a2b2e, 0xffe8e8e4, KnobStyle::capped,    0xff2a2a2a, { 0xffd8d8d2 }, 0xff141414, false },
        { "Moog Polymoog",        0xff191919, 0xff3a3a3a, 0xffeeeeea, 0xff9a9a96, 0xff6e4a2e, 0xffeeeeea, KnobStyle::capped,    0xff2a2a2a, { 0xffeeeeea }, 0xff141414, false },
        { "Fairlight CMI",        0xffe6e2d6, 0xffb8b3a5, 0xff2a2824, 0xff6a6660, 0xff4a7aa0, 0xfff6f4ee, KnobStyle::capped,    0xff2a2a2a, { 0xff3a3a3a }, 0xff141414, false },
        { "Hammond Novachord",    0xff4a2e1c, 0xff2e1c10, 0xfff1e6cc, 0xffc8b48c, 0xff2a1a0e, 0xfff1e6cc, KnobStyle::skirted,   0xff2a1c12, {}, 0xfff3e2a6, false },
    };
    // every other plugin after a real box: its family's paint, ink and knobs
    static const std::vector<F> families {
        // instruments: their own cases and panels
        { "Roland Juno-60", 0xff1c1d1f, 0xff3a3b3e, 0xffe8e8e4, 0xff9a9a96, 0xff2a2b2e, 0xffe8e8e4, KnobStyle::capped, 0xff2a2a2a, { 0xffe8e2d0, 0xffd86a22, 0xff2f8ac8 }, 0xff141414, false },
        { "Roland Juno-106", 0xff1a1b1d, 0xff38393c, 0xffe8e8e4, 0xff9a9a96, 0xffc8cacc, 0xff141414, KnobStyle::capped, 0xff2a2a2a, { 0xffeeeeea, 0xffc33a32 }, 0xff141414, false },
        { "TR-606", 0xffc2c4c6, 0xff8c8e90, 0xff141414, 0xff44464a, 0xff1a1a1a, 0xffe8eaec, KnobStyle::capped, 0xff1a1a1a, { 0xffd86a22, 0xff2a2a2a }, 0xff141414, true },
        { "Bass Line", 0xffc2c4c6, 0xff8c8e90, 0xff141414, 0xff44464a, 0xff1a1a1a, 0xffe8eaec, KnobStyle::capped, 0xff1a1a1a, { 0xff1a1a1a }, 0xff141414, true },
        { "Juno Six", 0xff1c1d1f, 0xff3a3b3e, 0xffe8e8e4, 0xff9a9a96, 0xff2a2b2e, 0xffe8e8e4, KnobStyle::capped, 0xff2a2a2a, { 0xffe8e2d0, 0xffd86a22 }, 0xff141414, false },
        { "Prophet Five", 0xff141414, 0xff323232, 0xffe6e6e2, 0xff94948f, 0xff6a3e1e, 0xfff2e6d6, KnobStyle::skirted, 0xff111111, {  }, 0xff141414, false },
        { "Model 72|Bass Pedals", 0xff151515, 0xff333333, 0xffeeeeea, 0xff9a9a96, 0xff6a3e1e, 0xfff2e6d6, KnobStyle::skirted, 0xff111111, {  }, 0xff141414, false },
        { "Twenty-Six", 0xff4b5e73, 0xff33424f, 0xffeef0f2, 0xffb0bcc8, 0xff2c3946, 0xffeef0f2, KnobStyle::capped, 0xff2a2a2a, { 0xffeeeeea, 0xffd86a22, 0xff2f5fae }, 0xff141414, false },
        { "VCS3", 0xffe8e4d8, 0xffb4b0a4, 0xff1a1a1a, 0xff4a4a48, 0xff6a3e1e, 0xfff2e6d6, KnobStyle::capped, 0xff1a1a1a, { 0xffc33a32, 0xff2f5fae, 0xff3b8a4a, 0xff1a1a1a, 0xff8a8e92 }, 0xff141414, false },
        { "Dream Machine", 0xff2a2c30, 0xff46484c, 0xffe8e8e4, 0xff9a9a96, 0xff1a1a1c, 0xffe8e8e4, KnobStyle::capped, 0xff2a2a2a, { 0xffc33a32, 0xff3b8a4a, 0xff2f5fae, 0xffd8b43a, 0xffe8e8e4 }, 0xff141414, false },
        { "FM 7", 0xff2a241e, 0xff4a4038, 0xffe8e4dc, 0xff9a948a, 0xff3aa88a, 0xff141414, KnobStyle::capped, 0xff2a2a2a, { 0xff8e9196 }, 0xff141414, false },
        { "Compact Organ|Combo Compact", 0xffb0281e, 0xff7a1812, 0xfff2f2ee, 0xffe8c4c0, 0xffcfd2d4, 0xff141414, KnobStyle::capped, 0xff1a1a1a, { 0xfff2efe6 }, 0xff141414, false },
        { "Continental", 0xffd0501e, 0xff923612, 0xfff6f2ea, 0xffecc8b4, 0xff1a1a1a, 0xfff6f2ea, KnobStyle::capped, 0xff1a1a1a, { 0xfff2efe6, 0xff1a1a1a }, 0xff141414, false },
        { "Tonewheel Organ|B-3 Organ", 0xff4a2c18, 0xff2c180a, 0xfff2e6d0, 0xffc8b08c, 0xff2a180c, 0xfff2e6d0, KnobStyle::chicken, 0xff1a1a1a, {  }, 0xfff3e2a6, false },
        { "Music Master|Tape 400", 0xff5a3a22, 0xff3a2414, 0xfff0dfc0, 0xffc4a880, 0xffe8e0cc, 0xff2a1a10, KnobStyle::chicken, 0xff1a1a1a, {  }, 0xfff3e2a6, false },
        { "Clavi D6", 0xff1a1a1a, 0xff3a3a3a, 0xffe6e6e2, 0xff94948f, 0xffcfd2d4, 0xff141414, KnobStyle::capped, 0xff1a1a1a, { 0xffc33a32, 0xfff2efe6 }, 0xff141414, false },
        { "Pianet", 0xff2a1e14, 0xff46342a, 0xfff0e6d6, 0xffbcae98, 0xff1a1410, 0xfff0e6d6, KnobStyle::capped, 0xff1a1a1a, { 0xff1a1a1a }, 0xff141414, false },
        { "Stage 73", 0xff1a1a1a, 0xff3a3a3a, 0xffe6e6e2, 0xff94948f, 0xffc4c7ca, 0xff141414, KnobStyle::capped, 0xff1a1a1a, { 0xff1a1a1a }, 0xff141414, false },
        { "RS-202", 0xff1c1d1f, 0xff3a3b3e, 0xffe8e8e4, 0xff9a9a96, 0xffbfc2c4, 0xff141414, KnobStyle::capped, 0xff2a2a2a, { 0xffe8e2d0 }, 0xff141414, false },
        { "String Ensemble", 0xff1e1a16, 0xff3e3630, 0xfff0e6d6, 0xffbcae98, 0xffd06a1e, 0xff141414, KnobStyle::capped, 0xff1a1a1a, { 0xffd06a1e, 0xfff2efe6 }, 0xff141414, false },
        { "Reed 200", 0xffcbb894, 0xff9a8a68, 0xff2a2016, 0xff5a4a36, 0xff3a2c1e, 0xfff2e8d4, KnobStyle::capped, 0xff1a1a1a, { 0xff1a1a1a }, 0xff141414, false },
        { "Clavioline", 0xffe4d6b8, 0xffb0a084, 0xff3a2a1c, 0xff7a6650, 0xff6a3e1e, 0xfff4ecdc, KnobStyle::chicken, 0xff2a1c12, {  }, 0xfff3e2a6, false },
        { "Synare 3", 0xff2a3440, 0xff1a222c, 0xffe6eaf0, 0xffa0aab6, 0xffc4c7ca, 0xff141414, KnobStyle::capped, 0xff1a1a1a, { 0xffc4c7ca }, 0xff141414, false },
        { "Muse Pianos", 0xff0e0e0e, 0xff2a2a2a, 0xffe8e0c8, 0xffa89e86, 0xff1a1a1a, 0xffd8b45a, KnobStyle::skirted, 0xff111111, {  }, 0xff141414, false },
        { "Electric Grand", 0xff1e1a18, 0xff3e3632, 0xffe8e4dc, 0xff9a948a, 0xff4a3a2e, 0xffe8e4dc, KnobStyle::capped, 0xff1a1a1a, { 0xff1a1a1a }, 0xff141414, false },
        { "Laptop Diary", 0xffcdd0d3, 0xff9a9da0, 0xff2a2c2e, 0xff6a6d70, 0xff2a2c2e, 0xffe8eaec, KnobStyle::aluminium, 0xff3a3c3e, {  }, 0xff141414, true },
        { "Scrappy Guitar", 0xff16171a, 0xff34363a, 0xffd8d8d4, 0xff8c8c88, 0xffc4c7ca, 0xff141414, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "Faded Tape|Live to Tape|Tretow Varispeed|Tape Loop|Tape Flanger|Hand Phaser", 0xffa9aba7, 0xff777975, 0xff161616, 0xff444442, 0xff3e403c, 0xfff0f0ea, KnobStyle::capped, 0xff2a2a2a, { 0xff4a4c50 }, 0xfff3e7c2, true },
        { "Room Crush", 0xff4e6472, 0xff364652, 0xfff0f2f2, 0xffc0cad0, 0xff2c3a44, 0xfff0f2f2, KnobStyle::skirted, 0xff111111, {  }, 0xfff3e7c2, false },
        { "Listen Mic", 0xff6d757b, 0xff4b5156, 0xffe9ebec, 0xffb4babe, 0xff40464b, 0xffe9ebec, KnobStyle::capped, 0xff2a2a2a, { 0xffc33a32, 0xff2f5fae, 0xffe0e0dc }, 0xff141414, false },
        { "Echo Chamber|Studio Mics|Feedback|Gold Star Chamber|Hollywood Chamber", 0xff7a8478, 0xff545c52, 0xfff2f2ec, 0xffc8cec4, 0xff3a423a, 0xfff2f2ec, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, false },
        { "Spring Reverb|Western Twang", 0xffb89a62, 0xff7f6a40, 0xff2a1e10, 0xff5a4428, 0xff6e2a1c, 0xfff3ead6, KnobStyle::chicken, 0xff3a2618, {  }, 0xfff3e7c2, false },
        { "Heater Spring|Deacy", 0xff6a4a30, 0xff46301e, 0xfff0e2c8, 0xffc4ae8c, 0xff2e1e12, 0xfff0e2c8, KnobStyle::chicken, 0xff1a1a1a, {  }, 0xfff3e7c2, false },
        { "Plank Echo|Brighton Rock Delay|Dual Delay", 0xff1c1c1c, 0xff3a3a3a, 0xffe8e8e4, 0xff94948f, 0xff3a6a4a, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff2a2a2a }, 0xfff3e7c2, false },
        { "Water Tank", 0xff8a8e8a, 0xff5e625e, 0xff141414, 0xff3a3e3a, 0xff3a3e3a, 0xfff0f0ea, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, true },
        { "Tuner", 0xff2a2c2e, 0xff46484a, 0xffe6e8ea, 0xff9a9c9e, 0xff8a2a22, 0xfff2f2ee, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "Car Radio", 0xffcfd2d4, 0xff8e9193, 0xff141414, 0xff444648, 0xff141414, 0xffe6e8ea, KnobStyle::capped, 0xff111111, { 0xffe6e6e2 }, 0xfff3e7c2, true },
        { "Transcription Disc", 0xff2a2420, 0xff4a4038, 0xfff0e6d6, 0xffbcae98, 0xff7a2a22, 0xfff0e6d6, KnobStyle::chicken, 0xff1a1a1a, {  }, 0xfff3e7c2, false },
        { "Cheap Speaker", 0xff8e9196, 0xff606368, 0xff141414, 0xff34363a, 0xff141414, 0xffe6e6e2, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Triple Bass Rig", 0xff141414, 0xff323232, 0xffd9b25a, 0xff9a8250, 0xffd9b25a, 0xff141414, KnobStyle::capped, 0xff141414, { 0xffd2b05a }, 0xfff3e7c2, false },
        { "Lonerism Vocal|Bellamy Vocal|Elfman Vocal|Croon Vocal", 0xff1d1e20, 0xff3a3b3e, 0xffd8d8d4, 0xff8c8c88, 0xffb9bcbd, 0xff111111, KnobStyle::skirted, 0xff111111, {  }, 0xfff3e7c2, false },
        { "Barker Bus|Vatos Kit|60s Kit|Hard to Explain|Orchestra Bus|Mystic Horns|Wall of Sound|Synth Room|Polar Acoustic|Mic Pair", 0xff5d625f, 0xff3d413f, 0xfff0ece0, 0xffb8b4a8, 0xff3a3e3c, 0xfff0ece0, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, false },
        { "Warmer|Soaked", 0xff1e1f21, 0xff3a3b3e, 0xffd8d8d4, 0xff8c8c88, 0xff6a2626, 0xffeeeeea, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "Diamond Vibrato", 0xff3a8a9a, 0xff246070, 0xfff2f2ee, 0xffc4e0e4, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Holy Grail", 0xffc8cacc, 0xff8a8c8e, 0xffb02a26, 0xff3a3a3c, 0xffc8a24a, 0xff141414, KnobStyle::capped, 0xff111111, { 0xff1a1a1a }, 0xff141414, true },
        { "Auto-Pan", 0xff2f8a8a, 0xff1e6060, 0xfff2f2ee, 0xffc4e0e0, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Hall|Shimmer|Cloud", 0xff2a3a6a, 0xff1a284a, 0xfff2f2ee, 0xffb8c4e0, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Reverse Reverb", 0xff4a2e6a, 0xff301c4a, 0xfff2f2ee, 0xffccc0e0, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Harmonizer", 0xffc03a7a, 0xff8a2456, 0xfff2f2ee, 0xffe8c0d4, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Lo-Fi Crush", 0xff3a3c3a, 0xff5a5c5a, 0xff9ae070, 0xff6a9a50, 0xff9ae070, 0xff141414, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Looper", 0xffd8dadc, 0xffa0a2a4, 0xff141414, 0xff4a4c4e, 0xff141414, 0xffd8dadc, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Low Drive|Twang Booster|Octave Bass", 0xff2a5a3a, 0xff1a3c26, 0xfff2f2ee, 0xffc4dccc, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Opto Comp", 0xffd0a02a, 0xff94701a, 0xff141414, 0xff3a3018, 0xff141414, 0xffd0a02a, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Ping-Pong Delay|Rhythm Taps", 0xff1e6aa8, 0xff144a78, 0xfff2f2ee, 0xffbcd4ea, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Bigsby", 0xffc4c7ca, 0xff8a8d90, 0xff141414, 0xff44464a, 0xff141414, 0xffe8eaec, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, true },
        { "6120 Voice", 0xffd2702a, 0xff94481a, 0xfff6eedc, 0xffeed8b8, 0xff2a1a10, 0xfff6eedc, KnobStyle::capped, 0xffd8b45a, { 0xffd8b45a }, 0xfff3e7c2, false },
        { "Ricky 330", 0xffc0281e, 0xff7a1410, 0xfff6f2ea, 0xffe8c8c0, 0xfff2efe6, 0xff141414, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xfff3e7c2, false },
        { "Red Special", 0xff6a1e16, 0xff401008, 0xfff2e6d6, 0xffccb49c, 0xff141414, 0xfff2e6d6, KnobStyle::capped, 0xff141414, { 0xffcfd2d4 }, 0xfff3e7c2, false },
        { "P-Bass Voice|Position 4", 0xff3a1e10, 0xff1e0e06, 0xfff6eedc, 0xffd8c4a0, 0xfff2efe6, 0xff141414, KnobStyle::capped, 0xff141414, { 0xffcfd2d4 }, 0xfff3e7c2, false },
        { "Bass Voice", 0xff7a3e1c, 0xff4a2410, 0xfff6eedc, 0xffd8c4a0, 0xff2a1a10, 0xfff6eedc, KnobStyle::capped, 0xffd8b45a, { 0xffd8b45a }, 0xfff3e7c2, false },
        { "Upright Slap|Owen Upright", 0xff8a4a1e, 0xff5a2e10, 0xfff6eedc, 0xffdcc4a0, 0xff2a1a10, 0xfff6eedc, KnobStyle::chicken, 0xff1a1a1a, {  }, 0xfff3e7c2, false },
        { "Quad Tracker|Guitar Orchestra|Pad Chopper", 0xff1b1b1c, 0xff3a3a3b, 0xffe9e6dc, 0xffa9a69c, 0xff2b2b2c, 0xffe9e6dc, KnobStyle::skirted, 0xff111111, {  }, 0xfff3e7c2, false },
        { "Deluxe Reverb|Deluxe Head|Showman AB763|Showman Head|Fender 1x12|Fender 2x12 JBL|Fender 2x15|Hot Rod Deluxe|Hot Rod DeVille|Harmonic Trem", 0xff1b1b1c, 0xff3a3a3b, 0xffe9e6dc, 0xffa9a69c, 0xff2b2b2c, 0xffe9e6dc, KnobStyle::skirted, 0xff111111, {  }, 0xfff3e7c2, false },
        { "Twin Reverb|Twin Head|Silverface Twin|Bassman 50", 0xffc5c7c8, 0xff8f9192, 0xff1c2a4a, 0xff4a5a7a, 0xff1e3a6a, 0xfff2f2ee, KnobStyle::skirted, 0xff111111, {  }, 0xfff3e7c2, true },
        { "Bassman 6G6-B|Bassman Head|Blonde Bassman|Bassman 2x12|Bassman 4x10", 0xffb89a62, 0xff7f6a40, 0xff2a1e10, 0xff5a4428, 0xff6e2a1c, 0xfff3ead6, KnobStyle::chicken, 0xff3a2618, {  }, 0xfff3e7c2, false },
        { "JTM45|JTM45 Head|Plexi Head|JCM800 Head|JCM900 Head|JCM900|Greenback 4x12|G12T-75 4x12|Time Modulator|Foxton Bass", 0xffc9a34a, 0xff8a6e2a, 0xff161412, 0xff4a3e2a, 0xff141414, 0xffd9b25a, KnobStyle::capped, 0xff141414, { 0xffd2b05a }, 0xfff3e7c2, true },
        { "AC30 Top Boost|AC30 Head|AC30 Normal|AC30 FWN|AC50|AC100|AC100 Head|AC100 Bass|Vox 2x12 Blue|Vox 4x12 Horns|DC30 Head|Matchless 2x12", 0xff7a4e2c, 0xff4a2e18, 0xfff1e4c8, 0xffc8b088, 0xff2a1a10, 0xfff1e4c8, KnobStyle::chicken, 0xffece2cc, {  }, 0xfff3e7c2, false },
        { "T60 Bass|UL730", 0xff6e6f6c, 0xff4a4b48, 0xfff1eee6, 0xffc0beb6, 0xff2a2a28, 0xfff1eee6, KnobStyle::chicken, 0xffece2cc, {  }, 0xfff3e7c2, false },
        { "Hiwatt Head|DR103 Bass|DR103 Custom 100|Hiwatt 4x12|Selmer TB50|Traynor YBA-1", 0xff141414, 0xff323232, 0xffeeeeea, 0xff9a9a96, 0xff222222, 0xffeeeeea, KnobStyle::capped, 0xff111111, { 0xffb8b8b4 }, 0xfff3e7c2, false },
        { "Mark IIB|Mark IIB Head|Rectifier Head|Rectified|Mesa 1x12 EV|Mesa 4x12 V30|VH4|Wizard Head|Wizard MC|Wizard 2x12|ODS Head|Valve Jr Celestion|Valve Jr V30", 0xff121212, 0xff303030, 0xffe6e6e2, 0xff94948f, 0xff1e1e1e, 0xffe6e6e2, KnobStyle::capped, 0xff0e0e0e, { 0xff2a2a2a }, 0xfff3e7c2, false },
        { "SLO Head", 0xffcfd2d4, 0xff8e9193, 0xff141414, 0xff444648, 0xff141414, 0xffe6e8ea, KnobStyle::capped, 0xff111111, { 0xff222222 }, 0xfff3e7c2, true },
        { "EH-150|EH-150 Head|EH-150 1x12|Archtop", 0xff5a3a22, 0xff3a2414, 0xfff0dfc0, 0xffc4a880, 0xff2e1c0e, 0xfff0dfc0, KnobStyle::skirted, 0xff2a1a10, {  }, 0xfff3e7c2, false },
        { "Magnatone Custom|Magnatone Head|Magnatone 1x15", 0xffe6dcc4, 0xffb0a688, 0xff3a2a1c, 0xff7a6650, 0xff7a3a26, 0xfff4ecdc, KnobStyle::skirted, 0xff1a1a1a, {  }, 0xfff3e7c2, false },
        { "SVT Bass|SVT 8x10|B-15 1x15", 0xffc4c7ca, 0xff8e9194, 0xff1a2440, 0xff4a5470, 0xff1e2c52, 0xfff0f0ec, KnobStyle::skirted, 0xff111111, {  }, 0xfff3e7c2, true },
        { "Jazz Chorus", 0xff1a1a1b, 0xff3a3a3b, 0xffe6e6e2, 0xff94948f, 0xffbfc2c4, 0xff141414, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "Backstage 30|Practice Amp Vocal|HD-130|HD-130 Head|Music Man 2x12", 0xff1c1c1c, 0xff3a3a3a, 0xffe6e6e2, 0xff94948f, 0xff8a8c8e, 0xff141414, KnobStyle::capped, 0xff111111, { 0xffb8b8b4 }, 0xfff3e7c2, false },
        { "DS-1|OS-2", 0xffe0702a, 0xffa04e1a, 0xff141414, 0xff3a2418, 0xff141414, 0xffe0702a, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Blues Driver|Vibrato", 0xff2f5fae, 0xff1e3f76, 0xfff2f2ee, 0xffc0cce0, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "CE-2 Chorus|Dimension", 0xff7ab8d8, 0xff4a8aaa, 0xff141414, 0xff2a4a5a, 0xff141414, 0xff7ab8d8, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "TR-2 Tremolo|Slow Gear", 0xff3a8a5a, 0xff24603c, 0xfff2f2ee, 0xffc4e0cc, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "DD-3|GE-7 EQ", 0xffe8e8e4, 0xffb0b0ac, 0xff141414, 0xff4a4a48, 0xff141414, 0xffe8e8e4, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Slicer", 0xff6a4a9a, 0xff4a3070, 0xfff2f2ee, 0xffccc0e0, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "CE-1 Chorus Ensemble|DM-1 Delay", 0xffb8babc, 0xff7e8082, 0xff141414, 0xff44464a, 0xff141414, 0xffe8e8e4, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, true },
        { "Phase 90", 0xffe8762a, 0xffa8501a, 0xff141414, 0xff3a2418, 0xff141414, 0xffe8762a, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Dyna Comp", 0xffc4302a, 0xff8a1e1a, 0xfff2f2ee, 0xffe0c0bc, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Distortion+", 0xffe8c22a, 0xffa8881a, 0xff141414, 0xff3a3018, 0xff141414, 0xffe8c22a, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Micro Amp|Analog Delay", 0xff8e9196, 0xff606368, 0xff141414, 0xff34363a, 0xff141414, 0xffe6e6e2, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, true },
        { "Flanger", 0xff4a2e6a, 0xff301c4a, 0xfff2f2ee, 0xffccc0e0, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Smart Gate|Digital Delay", 0xff1a1a1a, 0xff3a3a3a, 0xffe6e6e2, 0xff94948f, 0xff8a2a2a, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff2a2a2a }, 0xff141414, false },
        { "Big Muff|Muff R22|Freeze|POG|HOG|Small Stone", 0xffc8cacc, 0xff8a8c8e, 0xffb02a26, 0xff3a3a3c, 0xff141414, 0xffe6e6e2, KnobStyle::capped, 0xff111111, { 0xff1a1a1a }, 0xff141414, true },
        { "Electric Mistress|Small Clone|Memory Man", 0xffc8cacc, 0xff8a8c8e, 0xff2a4a8a, 0xff3a3a3c, 0xff2a4a8a, 0xfff2f2ee, KnobStyle::capped, 0xff111111, { 0xff1a1a1a }, 0xff141414, true },
        { "Fuzz Face|First Fuzz|Rangemaster|Treble Booster|Tone Bender|Octavia|Harmonic Percolator|IVP Preamp|Power Boost", 0xff7a7e80, 0xff4e5254, 0xfff2f2ee, 0xffc8ccce, 0xff2a2c2e, 0xfff2f2ee, KnobStyle::capped, 0xff111111, { 0xff1a1a1a }, 0xff141414, false },
        { "TS-808|Dead Battery TS", 0xff3a9a5a, 0xff246a3c, 0xfff2f2ee, 0xffc4e4cc, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Centaur|EP Booster", 0xffc8a24a, 0xff8a6e2a, 0xff1a1410, 0xff4a3a24, 0xff1a1410, 0xffd8b45a, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, true },
        { "RAT|Jekyll & Hyde", 0xff141414, 0xff343434, 0xffe6e6e2, 0xff94948f, 0xff141414, 0xffe6e6e2, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Whammy|Drop Tune|Wah", 0xffb8241e, 0xff7a1612, 0xfff2f2ee, 0xffe0c0bc, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "DL4", 0xff3a7a2a, 0xff24501a, 0xfff2f2ee, 0xffc4dcbc, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Fuzz Factory", 0xff2a6a8a, 0xff1a4a62, 0xfff2e8a0, 0xffc8d4dc, 0xffd8a030, 0xff141414, KnobStyle::capped, 0xff141414, { 0xffd8a030 }, 0xff141414, false },
        { "Envelope Filter|Bi-Phase", 0xff1a1a1a, 0xff3a3a3a, 0xffd8b45a, 0xff9a8250, 0xffd8b45a, 0xff141414, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "Uni-Vibe|Deja Vibe", 0xff2a2a2c, 0xff4a4a4c, 0xffe6e6e2, 0xff94948f, 0xff8a8c8e, 0xff141414, KnobStyle::skirted, 0xff111111, {  }, 0xff141414, false },
        { "Hotcake|Mudhoney", 0xff6a3a8a, 0xff4a2462, 0xfff2f2ee, 0xffd4c4e0, 0xff141414, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Ring Mod", 0xff8ab0c8, 0xff5a809a, 0xff141414, 0xff2a3a48, 0xff141414, 0xff8ab0c8, KnobStyle::capped, 0xff141414, { 0xff1a1a1a }, 0xff141414, false },
        { "Black Box EQ", 0xff6b7d8c, 0xff4a5865, 0xfff3f2ec, 0xffc9d0d6, 0xff4a5865, 0xfff3f2ec, KnobStyle::chicken, 0xff151515, {  }, 0xfff3e7c2, false },
        { "FET Limiter|Immunity Vocal|Black Box Limiter", 0xff1d1e20, 0xff3a3b3e, 0xffd8d8d4, 0xff8c8c88, 0xffb9bcbd, 0xff111111, KnobStyle::skirted, 0xff111111, {  }, 0xfff3e7c2, false },
        { "TG Strip|TG Channel|RS124|Studio EQ|Valve Desk|Valve Mixer|Gold Star Desk|Vari-Mu|Twin-Track Tape|Grit Comp|Doubler|Varispeed", 0xff7a8478, 0xff545c52, 0xfff2f2ec, 0xffc8cec4, 0xff3a423a, 0xfff2f2ec, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, false },
        { "EMT Plate|Plate 140|Strawberry Plate", 0xff8a9088, 0xff5e645c, 0xff141414, 0xff3a3e38, 0xff3a3e38, 0xfff0f0ea, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, false },
        { "Vocal Chain", 0xff5f7486, 0xff43535f, 0xfff1f1ee, 0xffc4ccd2, 0xff3c4a55, 0xfff1f1ee, KnobStyle::capped, 0xff6e7175, { 0xffb2443a, 0xff3e6aa8, 0xff8a8e92 }, 0xfff3e7c2, false },
        { "Helios Channel|Plank Console|Channel Dynamics", 0xff4e5a62, 0xff34404a, 0xfff1f1ee, 0xffc0c8ce, 0xff2a3238, 0xfff1f1ee, KnobStyle::capped, 0xff2a2a2a, { 0xff8a8e92, 0xffb2443a }, 0xfff3e7c2, false },
        { "Studer 169|A800 Bus|Analog Tape|Multitrack Tape|Tape Machine", 0xffb6b8b4, 0xff84868a, 0xff141414, 0xff3e4044, 0xff3e4044, 0xfff0f0ec, KnobStyle::capped, 0xff2a2a2a, { 0xff4a4c50 }, 0xff141414, true },
        { "Neotek Channel", 0xff3a4a6a, 0xff24304a, 0xfff0f2f6, 0xffb4bed0, 0xff1e2a40, 0xfff0f2f6, KnobStyle::capped, 0xff2a2a2a, { 0xffd8d8d2, 0xffc33a32, 0xff2f5fae }, 0xff141414, false },
        { "Ampex 350|Ampex 351|Tape Echo|Three-Head Echo|Backwards", 0xff7e8286, 0xff54585c, 0xfff2f2ee, 0xffc8cccf, 0xff3a3e42, 0xfff2f2ee, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, false },
        { "V72 V76", 0xff8a8e8a, 0xff5e625e, 0xff141414, 0xff3a3e3a, 0xff3a3e3a, 0xfff0f0ea, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, false },
        { "Echo Pro|Kaoss Pad|Nonlin|Level Loc", 0xff1d1e20, 0xff3a3b3e, 0xffdcdde0, 0xff8a8d90, 0xff2a2c2e, 0xffdcdde0, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "Space Station|Ford Space Station", 0xff6a4a32, 0xff46301e, 0xfff0e6d6, 0xffc4b49c, 0xff2a1c12, 0xfff0e6d6, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "AMS 15-80S", 0xff1f2d4a, 0xff314366, 0xffe6eaf2, 0xff9aa6bd, 0xff121b2e, 0xffe6eaf2, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "Chorus Echo|RE-301", 0xff1c1c1c, 0xff3a3a3a, 0xffe8e8e4, 0xff94948f, 0xff3a6a4a, 0xfff2f2ee, KnobStyle::capped, 0xff141414, { 0xff2a2a2a }, 0xfff3e7c2, false },
        { "Echorec", 0xffb0b4b6, 0xff7a7e80, 0xff1a1a1a, 0xff444648, 0xff8a2a22, 0xfff2f2ee, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, true },
        { "Copicat", 0xff2a2a2a, 0xff4a4a4a, 0xffd8b45a, 0xff9a8250, 0xffd8b45a, 0xff141414, KnobStyle::skirted, 0xff151515, {  }, 0xfff3e7c2, false },
        { "Revox Loop|Repro Head", 0xffb8babc, 0xff7e8082, 0xff141414, 0xff44464a, 0xff2a2c2e, 0xffe8e8e4, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, true },
        { "Rotary Speakers|Rotor", 0xff5a3a22, 0xff3a2414, 0xfff0dfc0, 0xffc4a880, 0xff2e1c0e, 0xfff0dfc0, KnobStyle::chicken, 0xff1a1a1a, {  }, 0xfff3e7c2, false },
        { "Kit Crush 165A", 0xff1a1a1a, 0xff3a3a3a, 0xffe0e0dc, 0xff94948f, 0xffc4c6c7, 0xff141414, KnobStyle::aluminium, 0xff1a1a1a, {  }, 0xff141414, false },
        { "Phone Line", 0xff9a9c9a, 0xff6a6c6a, 0xff141414, 0xff3a3c3a, 0xff2a2c2a, 0xffeeeeea, KnobStyle::chicken, 0xff151515, {  }, 0xfff3e7c2, false },
        { "Talk Box", 0xff2a2a2a, 0xff4a4a4a, 0xffe6e6e2, 0xff94948f, 0xff8a8c8e, 0xff141414, KnobStyle::skirted, 0xff111111, {  }, 0xff141414, false },
    };
    for (auto* table : { &faces, &families })
        for (auto& f : *table)
            if (juce::StringArray::fromTokens (f.names, "|", "").contains (name))
            {
                hardwareFace = true;
                PLATE = Colour (f.plate); PLATE_EDGE = Colour (f.edge); INK = Colour (f.ink); INK_SOFT = Colour (f.inkSoft);
                titleBlock = Colour (f.block); titleInk = Colour (f.blockInk);
                knobStyle = f.knob;
                BAKELITE = Colour (f.cap); BAKELITE_HI = Colour (f.cap).brighter (0.25f);
                for (auto c : f.caps) caps.push_back (Colour (c));
                METER_FACE = Colour (f.meter);
                brushed = f.brushed;
                accent = highlightFor();
                return;
            }
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
    titleBlock = accent;
    applyHardwareFace (inst.info().name);
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

    rack = isRackMounted();
    setOpaque (rack || hardwareFace);     // (edge to edge: nothing behind it needs redrawing)
    if (rack) rackLayout();
    else if (! presets.empty())
    {
        // the full-size panel keeps its size; if its lowest controls reach the preset bar, the rows
        // close up a little (the gaps between them, not the controls) so the bar sits clear below
        float top = 1.0e9f, lowest = 0.0f;
        for (auto& it : items)
        {
            if (it.kind == Kind::divider) continue;
            const float t = it.kind == Kind::knob ? it.y - it.r : it.kind == Kind::selector ? it.y - 20.0f : it.y;
            const float b = it.kind == Kind::knob ? it.y + it.r + 22.0f : it.kind == Kind::selector ? it.y + 36.0f
                          : it.kind == Kind::sw ? it.y + 9.0f : it.kind == Kind::text ? it.y + it.size * 0.6f
                          : it.kind == Kind::radio ? it.y + (float) ((it.labels.size() + juce::jmax (1, it.cols) - 1) / juce::jmax (1, it.cols)) * it.rowH
                          : it.y + it.h;
            top = juce::jmin (top, t);
            // (only what's above the footer itself: the preset bar and its PRESET label, and [PEQUOD])
            const float hw = it.kind == Kind::knob ? it.r + 30.0f : it.kind == Kind::selector ? 50.0f : it.kind == Kind::sw ? 50.0f : it.w * 0.5f;
            const float cx = it.kind == Kind::knob || it.kind == Kind::selector || it.kind == Kind::sw ? it.x : it.x + it.w * 0.5f;
            const bool overBar = cx + hw > presetPrevRect().getX() - 60.0f && cx - hw < presetNextRect().getRight();
            const bool overTag = cx - hw < 120.0f;
            if (overBar || overTag) lowest = juce::jmax (lowest, b);
        }
        const float limit = (float) panelH - 50.0f;
        if (lowest > limit && lowest > top + 1.0f)
        {
            const float k = juce::jmax (0.8f, (limit - top) / (lowest - top));
            for (auto& it : items)
                it.y = top + (it.y - top) * k;
        }
    }
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
    // as the VST3 panel: continuous parameters count as one step - except one the panel shows as
    // a list of choices (a selector or a radio list): a stepped number reports no steps of its
    // own, so its choices say how many there are (else every click lands on the first or last)
    auto& ps = inst.info().params;
    const int own = juce::isPositiveAndBelow (p, (int) ps.size()) ? ps[(size_t) p].steps : 0;
    if (own > 0) return own;
    for (auto& it : items)
        if (it.p == p && (it.kind == Kind::radio || it.kind == Kind::selector) && it.labels.size() > 1)
            return it.labels.size() - 1;
    return 1;
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
    textBounds = getLocalBounds().toFloat();
    if (textLog != nullptr)
        drawFaceplate (g);       // (checking: every word logged)
    else
    {
        const float sc = juce::jmax (1.0f, g.getInternalContext().getPhysicalPixelScaleFactor());
        const int iw = juce::roundToInt ((float) getWidth() * sc), ih = juce::roundToInt ((float) getHeight() * sc);
        if (! plateCache.isValid() || plateCache.getWidth() != iw || plateCache.getHeight() != ih)
        {
            plateCache = juce::Image (juce::Image::ARGB, juce::jmax (1, iw), juce::jmax (1, ih), true);
            juce::Graphics ig (plateCache);
            ig.addTransform (juce::AffineTransform::scale (sc));
            drawFaceplate (ig);
            textBounds = getLocalBounds().toFloat();
        }
        g.drawImage (plateCache, getLocalBounds().toFloat());
    }

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
            case Kind::divider:  if (! rack) line (g, { 40.0f, it.y }, { (float) panelW - 40.0f, it.y }, 1.0f, PLATE_EDGE); break;
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
    if (rack) { if (presets.empty()) say (g, "[PEQUOD]", { 16.0f, h - 14.0f }, -1, 0, 10.0f, (hardwareFace ? titleInk : CREAM).withAlpha (0.6f), true); }
    else      say (g, "[PEQUOD]", { 38.0f, h - 36.0f }, -1, 0, 12.0f, Colour (0xffa8823a), true);
    if (! presets.empty())
    {
        drawPresetBar (g);
        if (presetOpen) drawPresetList (g);
    }
}

void PequodPanel::drawFaceplate (juce::Graphics& g) const
{
    const float w = (float) panelW, h = (float) panelH;
    if (rack)
    {
        // a rack faceplate: the plate edge to edge, the name block down the left in the accent
        g.setGradientFill (juce::ColourGradient (PLATE.brighter (0.05f), 0.0f, 0.0f, PLATE.darker (0.06f), 0.0f, h, false));
        g.fillAll();
        g.setColour (PLATE_EDGE);
        g.drawRect (getLocalBounds().toFloat(), 1.5f);
        const Rect block (0.0f, 0.0f, (float) rackTitleWidth, h);
        if (hardwareFace) drawPlateTexture (g, getLocalBounds().toFloat());
        g.setGradientFill (juce::ColourGradient (titleBlock.brighter (0.08f), 0.0f, 0.0f, titleBlock.darker (0.2f), 0.0f, h, false));
        g.fillRect (block);
        g.setColour (blackAlpha (60));
        g.fillRect (block.withX (block.getRight() - 2.0f).withWidth (2.0f));
        const float fitW = (float) rackTitleWidth - 28.0f;
        sayFit (g, title, { 16.0f, 30.0f }, fitW, 24.0f, hardwareFace ? titleInk : CREAM);
        sayFit (g, sub1, { 16.0f, 54.0f }, fitW, 10.5f, hardwareFace ? titleInk.withAlpha (0.8f) : CREAM);
        sayFit (g, sub2, { 16.0f, 68.0f }, fitW, 10.5f, hardwareFace ? titleInk.withAlpha (0.8f) : CREAM);
        return;
    }
    if (hardwareFace)
    {
        // the box's own front panel, edge to edge: its paint, a strip of its colour for the name, four screws
        g.setGradientFill (juce::ColourGradient (PLATE.brighter (0.06f), 0.0f, 0.0f, PLATE.darker (0.1f), 0.0f, h, false));
        g.fillAll();
        drawPlateTexture (g, getLocalBounds().toFloat());
        g.setColour (PLATE_EDGE);
        g.drawRect (getLocalBounds().toFloat(), 2.0f);
        // the name strip: as tall as it can be above the controls (52 px), shallower if they sit high
        float top = h;
        for (auto& it : items)
            top = juce::jmin (top, it.kind == Kind::knob ? it.y - it.r - 26.0f : it.kind == Kind::selector ? it.y - 46.0f
                                 : it.kind == Kind::divider ? h : it.y - 12.0f);
        const float bh = juce::jlimit (34.0f, 52.0f, top - 14.0f - 6.0f);
        const Rect band (0.0f, 14.0f, w, bh);
        g.setColour (titleBlock);
        g.fillRect (band);
        g.setColour (blackAlpha (70));
        g.fillRect (band.withY (band.getBottom()).withHeight (2.0f));
        textBounds = getLocalBounds().toFloat();
        float ts = juce::jmin (28.0f, bh * 0.54f);
        while (ts > 14.0f && juce::GlyphArrangement::getStringWidth (font (ts), title) > w * 0.5f) ts -= 1.0f;
        const float mid = band.getCentreY(), gap = juce::jmin (8.0f, bh * 0.16f);
        say (g, title, { 44.0f, mid }, -1, 0, ts, titleInk);
        const float room = w - 88.0f - juce::GlyphArrangement::getStringWidth (font (ts), title) - 24.0f;
        for (int k = 0; k < 2; ++k)
        {
            const auto& sub = k == 0 ? sub1 : sub2;
            float ss = juce::jmin (11.5f, bh * 0.24f);
            while (ss > 7.5f && juce::GlyphArrangement::getStringWidth (font (ss), sub) > room) ss -= 0.5f;
            const float sw = juce::GlyphArrangement::getStringWidth (font (ss), sub);
            sayFit (g, sub, { w - 44.0f - juce::jmin (sw, room), k == 0 ? mid - gap : mid + gap }, room, ss, titleInk.withAlpha (0.85f));
        }
        for (auto c : { Pt (16.0f, 8.0f), Pt (w - 16.0f, 8.0f), Pt (16.0f, h - 10.0f), Pt (w - 16.0f, h - 10.0f) })
        {
            circle (g, c + Pt (0.4f, 0.8f), 5.0f, blackAlpha (90));
            g.setGradientFill (juce::ColourGradient (Colour (0xffd8d8d4), c.x - 3.0f, c.y - 3.0f, Colour (0xff5a5a58), c.x + 3.0f, c.y + 4.0f, true));
            g.fillEllipse (c.x - 4.5f, c.y - 4.5f, 9.0f, 9.0f);
            const float a = slotTurn (c, (float) title.hashCode());
            line (g, polar (c, 3.4f, a), polar (c, 3.4f, a + pi), 1.4f, Colour (0xff222222));
        }
        return;
    }
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

/** The face's highlight (a switch that's on, a value's arc, the chosen radio button): the
    colour of its knob caps or its name strip, if either is a real colour, else an amber lamp. */
Colour PequodPanel::highlightFor() const
{
    std::vector<Colour> candidates (caps.begin(), caps.end());
    candidates.push_back (titleBlock);
    candidates.push_back (PLATE);
    for (auto c : candidates)
        if (c.getSaturation() > 0.35f && c.getBrightness() > 0.35f && std::abs (c.getPerceivedBrightness() - PLATE.getPerceivedBrightness()) > 0.12f)
            return c;
    for (auto c : candidates)
        if (c.getSaturation() > 0.35f && c.getBrightness() > 0.35f)
            return PLATE.getPerceivedBrightness() > 0.5f ? c.darker (0.4f) : c.brighter (0.3f);
    return Colour (0xffe0a030);
}

/** The preset display: a dark window, lit in the meter's colour (or amber, if the meters are black). */
Colour PequodPanel::displayBg() const
{
    return BAKELITE.getPerceivedBrightness() < 0.3f ? BAKELITE : Colour (0xff1c1a18);
}
Colour PequodPanel::displayInk() const
{
    return METER_FACE.getPerceivedBrightness() > 0.5f ? METER_FACE : Colour (0xffe0a030);
}

/** Text on the highlight: dark on a pale one, cream on a deep one. */
Colour PequodPanel::onAccent() const
{
    return accent.getPerceivedBrightness() > 0.62f ? Colour (0xff141414) : CREAM;
}

void PequodPanel::drawPlateTexture (juce::Graphics& g, Rect r) const
{
    if (! brushed) return;
    // brushed aluminium: fine horizontal grain
    juce::Random grain ((juce::int64) title.hashCode());
    for (float y = r.getY() + 1.0f; y < r.getBottom() - 1.0f; y += 1.5f)
    {
        g.setColour ((grain.nextBool() ? Colour (0xffffffff) : Colour (0xff000000)).withAlpha (0.015f + 0.03f * grain.nextFloat()));
        g.drawHorizontalLine ((int) y, r.getX(), r.getRight());
    }
}

//==============================================================================
void PequodPanel::drawKnob (juce::Graphics& g, const Item& it, int index) const
{
    if (hardwareFace && knobStyle != KnobStyle::bakelite)
    {
        drawHardwareKnob (g, it, index);
        return;
    }
    const bool dim = isDim (it);
    const bool big = it.r > 30.0f;
    const Pt c (it.x, it.y);
    const float r = it.r;
    auto fade = [this, dim] (Colour col) { return dim ? lerp (col, PLATE, 0.6f) : col; };
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

void PequodPanel::drawHardwareKnob (juce::Graphics& g, const Item& it, int index) const
{
    const bool dim = isDim (it);
    const bool big = it.r > 30.0f;
    const Pt c (it.x, it.y);
    const float r = it.r;
    auto fade = [this, dim] (Colour col) { return dim ? lerp (col, PLATE, 0.6f) : col; };
    const auto ink = fade (INK), inkSoft = fade (INK_SOFT);
    const float v = value (it.p);
    const float a0 = knobAngle (0.0f), a1 = knobAngle (v);

    // the scale printed round it
    const int ticks = big ? 11 : 11;
    for (int t = 0; t < ticks; ++t)
    {
        const float a = knobAngle ((float) t / (float) (ticks - 1));
        const bool lng = t == 0 || t == ticks - 1 || t == ticks / 2;
        line (g, polar (c, r + 4.0f, a), polar (c, r + (lng ? 10.0f : 7.0f), a), lng ? 1.6f : 1.0f, ink);
    }
    circle (g, c + Pt (0.8f, 3.0f), r, blackAlpha (dim ? 30 : 90));

    // which cap: by the knob's order on the panel, as a desk's colour code
    int knobIndex = 0;
    for (int k = 0; k < index && k < (int) items.size(); ++k) if (items[(size_t) k].kind == Kind::knob) ++knobIndex;
    const auto cap = caps.empty() ? BAKELITE : caps[(size_t) knobIndex % caps.size()];

    switch (knobStyle)
    {
        case KnobStyle::chicken:
        {
            // a black chicken-head: a round boss and a long tapering pointer with a white line
            circle (g, c, r * 0.7f, fade (Colour (0xff121212)));
            juce::Path head;
            const float L = r * 1.05f, W = r * 0.42f;
            head.startNewSubPath (0.0f, -L);
            head.quadraticTo (W, -L * 0.4f, W * 0.9f, r * 0.2f);
            head.quadraticTo (0.0f, r * 0.75f, -W * 0.9f, r * 0.2f);
            head.quadraticTo (-W, -L * 0.4f, 0.0f, -L);
            head.closeSubPath();
            g.setColour (fade (Colour (0xff181818)));
            g.fillPath (head, juce::AffineTransform::rotation (a1).translated (c));
            g.setColour (fade (Colour (0xff3a3a3a)));
            g.strokePath (head, juce::PathStrokeType (1.0f), juce::AffineTransform::rotation (a1).translated (c));
            line (g, polar (c, r * 0.1f, a1), polar (c, L * 0.92f, a1), 2.2f, fade (Colour (0xfff2f0ea)));
            break;
        }
        case KnobStyle::aluminium:
        {
            // a knurled aluminium knob with a black line
            g.setGradientFill (juce::ColourGradient (fade (Colour (0xffe6e7e8)), c.x - r * 0.6f, c.y - r * 0.7f, fade (Colour (0xff7b7d80)), c.x + r * 0.7f, c.y + r, true));
            g.fillEllipse (c.x - r, c.y - r, r * 2.0f, r * 2.0f);
            for (int k = 0; k < 36; ++k)
            {
                const float a = (float) k * pi / 18.0f + a1;
                line (g, polar (c, r * 0.88f, a), polar (c, r, a), 0.8f, fade (Colour (0xff6a6c6f)));
            }
            circle (g, c, r * 0.72f, fade (Colour (0xffc9cbcd)));
            line (g, polar (c, r * 0.15f, a1), polar (c, r * 0.95f, a1), big ? 3.0f : 2.2f, fade (cap));
            break;
        }
        case KnobStyle::capped:
        {
            // a grey or black body with a coloured cap and a white pointer line (a desk's knobs)
            circle (g, c, r, fade (BAKELITE));
            for (int k = 0; k < 20; ++k)
            {
                const float a = (float) k * pi / 10.0f + a1;
                line (g, polar (c, r * 0.82f, a), polar (c, r * 0.98f, a), 1.0f, fade (BAKELITE_HI));
            }
            circle (g, c, r * 0.62f, fade (cap));
            circle (g, c - Pt (r * 0.12f, r * 0.15f), r * 0.3f, fade (cap.brighter (0.25f)).withAlpha (0.35f));
            line (g, polar (c, r * 0.05f, a1), polar (c, r * 0.96f, a1), big ? 3.0f : 2.2f, fade (cap.getPerceivedBrightness() > 0.7f ? Colour (0xff1a1a1a) : Colour (0xfff4f2ec)));
            break;
        }
        case KnobStyle::skirted:
        default:
        {
            // a big black knob on a skirt: the skirt carries the pointer, polished metal edge
            circle (g, c, r, fade (Colour (0xff0e0e0e)));
            ring (g, c, r - 0.5f, 1.2f, fade (Colour (0xff8e8f90)));
            g.setGradientFill (juce::ColourGradient (fade (Colour (0xff3a3a3a)), c.x - r * 0.4f, c.y - r * 0.5f, fade (Colour (0xff0b0b0b)), c.x + r * 0.5f, c.y + r * 0.5f, true));
            g.fillEllipse (c.x - r * 0.66f, c.y - r * 0.66f, r * 1.32f, r * 1.32f);
            line (g, polar (c, r * 0.68f, a1), polar (c, r * 0.97f, a1), big ? 3.5f : 2.6f, fade (Colour (0xfff2f0ea)));
            break;
        }
    }
    if (v > 0.001f && ! big)
        polyline (g, 30, [&] (int k) { return polar (c, r + 2.0f, a0 + (a1 - a0) * (float) k / 30.0f); }, 1.6f, fade (accent).withAlpha (0.8f));
    if (hoverItem == index || dragItem == index)
        ring (g, c, r + 1.0f, 1.5f, accent);
    const float lf = big ? 15.0f : 11.0f, vf = big ? 13.0f : 11.0f;
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
    if (hardwareFace)
    {
        g.setColour (PLATE_EDGE.withAlpha (0.9f));
        g.drawRoundedRectangle (pill, 8.0f, 1.0f);
    }
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
    // where each position's label goes: side ones hang outward to clear the knob; crowded ones
    // get smaller, and if they still touch, every other one steps out
    struct L { Pt at; int ax; Rect box; };
    auto place = [&] (float size, bool stagger)
    {
        std::vector<L> ls;
        for (int k = 0; k < n && k < it.labels.size(); ++k)
        {
            const float a = selectorAngle (it, k);
            const float out = stagger && (k % 2 == 1) ? 12.0f : 0.0f;
            L l;
            if (n >= 4 && std::sin (a) > 0.6f)       { l.at = polar (c, r + 12.0f + out, a); l.ax = -1; }
            else if (n >= 4 && std::sin (a) < -0.6f) { l.at = polar (c, r + 12.0f + out, a); l.ax = 1; }
            else                                     { l.at = polar (c, r + 20.0f + out, a); l.ax = 0; }
            const float w = juce::GlyphArrangement::getStringWidth (font (size), it.labels[k]);
            const float hh = size * 0.9f;
            const float x = l.ax < 0 ? l.at.x : l.ax > 0 ? l.at.x - w : l.at.x - w * 0.5f;
            l.box = Rect (x, l.at.y - hh * 0.5f, w, hh);
            ls.push_back (l);
        }
        return ls;
    };
    auto clashes = [] (const std::vector<L>& ls)
    {
        for (size_t a = 0; a < ls.size(); ++a)
            for (size_t b = a + 1; b < ls.size(); ++b)
                if (ls[a].box.intersects (ls[b].box)) return true;
        return false;
    };
    float size = 10.0f;
    bool stagger = false;
    auto ls = place (size, stagger);
    while (clashes (ls) && size > 7.5f) { size -= 0.5f; ls = place (size, stagger); }
    if (clashes (ls)) { stagger = true; ls = place (size, stagger); }
    for (int k = 0; k < n; ++k)
    {
        const float a = selectorAngle (it, k);
        line (g, polar (c, r + 4.0f, a), polar (c, r + 9.0f, a), 1.5f, INK);
        if (k >= (int) ls.size()) continue;
        say (g, it.labels[k], ls[(size_t) k].at, ls[(size_t) k].ax, 0, size, k == current ? accent : INK_SOFT);
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
    // Drawn to scale: a rack's smaller meter is the same meter, its type kept readable
    // (and fewer numbers on the scale when it's small)
    const float s = it.h0 > 0.0f ? juce::jlimit (0.3f, 1.0f, it.h / it.h0) : 1.0f;
    const bool small = s < 0.8f;
    const float font = juce::jmax (8.5f, 11.0f * s);
    const Rect rect (it.x, it.y, it.w, it.h);
    const bool gr = it.meterKind == "gr";
    // the meter's own printing: dark on a lit cream face, pale on a black one
    const bool paleFace = METER_FACE.getPerceivedBrightness() > 0.5f;
    const Colour mInk = paleFace ? Colour (0xff1a120c) : Colour (0xffe8e4d8), mInkSoft = mInk.withAlpha (0.7f);
    const float value = juce::isPositiveAndBelow (it.m, (int) shown.size()) ? shown[(size_t) it.m] : (gr ? 0.0f : -25.0f);
    g.setColour (BAKELITE);
    g.fillRoundedRectangle (rect.expanded (5.0f * s), 6.0f * s);
    g.setColour (METER_FACE);
    g.fillRoundedRectangle (rect, 3.0f);
    const Pt pivot (rect.getCentreX(), rect.getBottom() + 28.0f * s);
    const float radius = rect.getHeight() + 2.0f * s;
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
    struct Mark { float v; const char* t; const char* tSmall; };
    static const Mark grMarks[] = { { 0, "0", "0" }, { 1, "", "" }, { 2, "2", "" }, { 3, "", "" }, { 5, "5", "5" }, { 7, "", "" },
                                    { 10, "10", "10" }, { 15, "", "" }, { 20, "20", "20" } };
    static const Mark vuMarks[] = { { -20, "20", "20" }, { -10, "10", "10" }, { -7, "7", "" }, { -5, "5", "5" }, { -3, "3", "" },
                                    { -2, "", "" }, { -1, "", "" }, { 0, "0", "0" }, { 1, "", "" }, { 2, "", "" }, { 3, "+3", "+3" } };
    const float redFrom = gr ? 10.0f : 0.0f, redTo = gr ? 20.0f : 3.0f;
    polyline (g, 20, [&] (int k) { return polar (pivot, radius - 22.0f * s, map (redFrom + (redTo - redFrom) * (float) k / 20.0f)); }, 4.0f * s, accent);
    const float lo = gr ? 0.0f : -20.0f, hi = gr ? 20.0f : 3.0f;
    polyline (g, 40, [&] (int k) { return polar (pivot, radius - 26.0f * s, map (lo + (hi - lo) * (float) k / 40.0f)); }, 1.2f, mInk);
    auto drawMarks = [&] (const Mark* m, int count)
    {
        for (int k = 0; k < count; ++k)
        {
            const float a = map (m[k].v);
            const bool lng = m[k].t[0] != 0;
            line (g, polar (pivot, radius - 26.0f * s, a), polar (pivot, radius - (lng ? 36.0f : 32.0f) * s, a), 1.2f, mInk);
            const char* t = small ? m[k].tSmall : m[k].t;
            if (t[0] != 0) say (g, t, polar (pivot, radius - 14.0f * s, a), 0, 0, font, mInk);
        }
    };
    if (gr) drawMarks (grMarks, (int) std::size (grMarks));
    else    drawMarks (vuMarks, (int) std::size (vuMarks));
    // its name (and VU / dB), between the scale and the strip
    if (small)
        say (g, it.label + (gr ? juce::String() : juce::String (" VU")), { rect.getCentreX(), rect.getBottom() - 10.0f * s - font * 0.75f }, 0, 0, juce::jmax (8.0f, font - 1.0f), mInkSoft);
    else
    {
        say (g, it.label, { rect.getCentreX(), rect.getBottom() - 34.0f * s }, 0, 0, 10.0f * s, mInkSoft);
        say (g, gr ? "dB" : "VU", { rect.getCentreX(), rect.getBottom() - 21.0f * s }, 0, 0, 10.0f * s, mInkSoft);
    }
    {
        juce::Graphics::ScopedSaveState ss (g);
        g.reduceClipRegion (rect.toNearestInt());
        line (g, pivot, polar (pivot, radius - 8.0f * s, map (value)), 2.0f, paleFace ? Colour (0xff1a120c) : Colour (0xffe0a030));
    }
    juce::Path strip;
    strip.addRoundedRectangle (rect.getX(), rect.getBottom() - 10.0f * s, rect.getWidth(), 10.0f * s, 3.0f, 3.0f, false, false, true, true);
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

namespace
{
    int radioRows (int n, int cols) { return (n + juce::jmax (1, cols) - 1) / juce::jmax (1, cols); }
}

void PequodPanel::drawRadio (juce::Graphics& g, const Item& it) const
{
    const int current = indexOf (it.p);
    for (int k = 0; k < it.labels.size(); ++k)
    {
        const int rows = radioRows (it.labels.size(), it.cols);
        const Rect r (it.x + (float) (k / rows) * it.w, it.y + (float) (k % rows) * it.rowH, it.w - 6.0f * (float) (it.cols > 1), it.rowH - 3.0f);
        const bool on = k == current;
        const bool hovered = r.contains (lastMouse) && isMouseOver();
        if (on || hovered)
        {
            g.setColour (on ? accent : lerp (PLATE, PLATE_EDGE, 0.35f));
            g.fillRoundedRectangle (r, 4.0f);
        }
        const Pt lc (r.getX() + 12.0f, r.getCentreY());
        ring (g, lc, 4.5f, 1.2f, on ? onAccent() : INK_SOFT);
        if (on) circle (g, lc, 2.6f, onAccent());
        say (g, it.labels[k], lc + Pt (12.0f, 0.0f), -1, 0, 12.5f, on ? onAccent() : INK);
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
// Rack-mounted panels

juce::Rectangle<float> PequodPanel::itemBounds (const Item& it) const
{
    const auto textW = [] (const juce::String& s, float size) { return 0.55f * size * (float) s.length(); };
    switch (it.kind)
    {
        case Kind::knob:
        {
            const bool big = it.r > 30.0f;
            const float hw = juce::jmax (it.r + 10.0f, 0.5f * textW (it.label, big ? 16.0f : 11.0f) + 4.0f);
            return Rect (it.x - hw, it.y - it.r - (big ? 32.0f : 28.0f), 2.0f * hw, 2.0f * it.r + (big ? 62.0f : 54.0f));
        }
        case Kind::selector:
        {
            // the side positions' labels hang outward: as wide as the longest of them
            float lw = 0.0f;
            for (auto& l : it.labels) lw = juce::jmax (lw, textW (l, 10.0f));
            const float hw = juce::jmax (it.r + 34.0f, it.r + 14.0f + (it.labels.size() >= 4 ? lw : lw * 0.5f));
            return Rect (it.x - hw, it.y - it.r - 30.0f, 2.0f * hw, 2.0f * it.r + 52.0f);
        }
        case Kind::sw:
        {
            const float lw = it.labels.size() >= 2 ? juce::jmax (textW (it.labels[0], 11.0f), textW (it.labels[1], 11.0f)) : 0.0f;
            return Rect (it.x - 23.0f - lw, it.y - 12.0f, 46.0f + 2.0f * lw, 24.0f);
        }
        case Kind::lamp:      return Rect (it.x - 9.0f, it.y - 9.0f, 26.0f + textW (it.label, 12.0f), 18.0f);
        case Kind::polar:
        case Kind::rotor:     return Rect (it.x - it.r - 16.0f, it.y - it.r - 20.0f, 2.0f * it.r + 32.0f, 2.0f * it.r + (it.kind == Kind::rotor ? 50.0f : 40.0f));   // (a rotor's caption under it)
        case Kind::radio:     return Rect (it.x, it.y, it.w * (float) it.cols, it.rowH * (float) juce::jmax (1, radioRows (it.labels.size(), it.cols)));
        case Kind::caption:   return Rect (it.x, it.y, it.w, juce::jmax (it.h, it.size * 2.6f));
        case Kind::irloader:  return Rect (it.x, it.y, it.w, juce::jmax (it.h, 30.0f));
        case Kind::meter: case Kind::tuner: case Kind::roomplan:
            return Rect (it.x, it.y, it.w, it.h).expanded (0.0f, 14.0f);
        case Kind::text:
        {
            const float w = textW (it.label, it.size), h = it.size * 1.4f;
            const float x = it.alignX < 0 ? it.x : it.alignX > 0 ? it.x - w : it.x - w / 2.0f;
            const float y = it.alignY < 0 ? it.y : it.alignY > 0 ? it.y - h : it.y - h / 2.0f;
            return Rect (x, y, w, h);
        }
        case Kind::divider: case Kind::unknown: break;
    }
    return {};
}

void PequodPanel::rackLayout()
{
    // 1. the controls as groups, in reading order: each control on its own, with any heading
    //    printed just above it travelling with it
    // 0. rack gear is compact: big knobs come down to the small size, meters and plans to rack height
    for (auto& it : items)
    {
        switch (it.kind)
        {
            case Kind::knob:     it.r = juce::jmin (it.r, 22.0f); break;
            case Kind::selector: it.r = juce::jmin (it.r, 18.0f); break;
            case Kind::polar:    it.r = juce::jmin (it.r, 30.0f); break;
            case Kind::rotor:    it.r = juce::jmin (it.r, 22.0f); break;
            case Kind::radio:    // a long list folds into columns of four
                it.cols = (it.labels.size() + 3) / 4; it.rowH = juce::jmin (it.rowH, 18.0f);
                break;
            case Kind::meter: case Kind::tuner:
                it.h0 = it.h;
                if (it.h > 72.0f) { it.w *= 72.0f / it.h; it.h = 72.0f; }
                break;
            case Kind::roomplan:
                if (it.h > 72.0f) { it.w *= 72.0f / it.h; it.h = 72.0f; }
                break;
            default: break;
        }
    }

    struct Group { Rect area; std::vector<int> items; float anchor = 0.0f; };
    std::vector<Rect> boxes (items.size());
    std::vector<int> controls, texts;
    for (int k = 0; k < (int) items.size(); ++k)
    {
        boxes[(size_t) k] = itemBounds (items[(size_t) k]);
        if (boxes[(size_t) k].isEmpty()) continue;
        (items[(size_t) k].kind == Kind::text ? texts : controls).push_back (k);
    }
    if (controls.empty() && texts.empty()) return;

    std::vector<Group> groups;
    for (int k : controls) groups.push_back ({ boxes[(size_t) k], { k } });
    for (int k : texts)
    {
        // a heading: the nearest control just under it that it overlaps sideways
        const auto& t = boxes[(size_t) k];
        int best = -1;
        float bestGap = 44.0f;
        for (int g = 0; g < (int) groups.size(); ++g)
        {
            const auto& c = boxes[(size_t) groups[(size_t) g].items.front()];
            const float gap = c.getY() - t.getBottom();
            if (gap > -6.0f && gap < bestGap && c.getHorizontalRange().intersects (t.getHorizontalRange()))
                { best = g; bestGap = gap; }
        }
        if (best >= 0)
        {
            // a heading centred over a pair or a row (Bi-Phase's PHASOR A over DEPTH and FEEDBACK): the controls
            // the same distance either side of it, on its line, go with it as one group
            {
                const auto& cb = items[(size_t) groups[(size_t) best].items.front()];
                const float reach = std::abs (boxes[(size_t) groups[(size_t) best].items.front()].getCentreX() - t.getCentreX()) + 12.0f;
                if (reach > 16.0f)
                    for (int g = 0; g < (int) groups.size(); ++g)
                    {
                        if (g == best || groups[(size_t) g].items.size() != 1) continue;
                        const auto& c = items[(size_t) groups[(size_t) g].items.front()];
                        const auto& cbox = boxes[(size_t) groups[(size_t) g].items.front()];
                        if (c.kind == Kind::text || std::abs (c.y - cb.y) > 4.0f) continue;
                        if (std::abs (cbox.getCentreX() - t.getCentreX()) > reach) continue;
                        auto& bg = groups[(size_t) best];
                        bg.area = bg.area.getUnion (cbox);
                        bg.items.push_back (groups[(size_t) g].items.front());
                        groups.erase (groups.begin() + g);
                        if (g < best) --best;
                        --g;
                    }
            }
            // a heading over a row of switches (Echorec's HEADS 1 2 3 4) keeps the whole row with it
            const auto& c0 = items[(size_t) groups[(size_t) best].items.front()];
            if (c0.kind == Kind::sw)
                for (bool grew = true; grew;)
                {
                    grew = false;
                    for (int g = 0; g < (int) groups.size(); ++g)
                    {
                        if (g == best || groups[(size_t) g].items.size() != 1) continue;
                        const auto& c = items[(size_t) groups[(size_t) g].items.front()];
                        const auto& a = groups[(size_t) g].area;
                        const auto& ab = groups[(size_t) best].area;
                        const float gapBetween = juce::jmax (a.getX() - ab.getRight(), ab.getX() - a.getRight());
                        if (c.kind == Kind::sw && std::abs (c.y - c0.y) < 4.0f && gapBetween < 40.0f)
                        {
                            auto& bg = groups[(size_t) best];
                            bg.area = bg.area.getUnion (a);
                            bg.items.push_back (groups[(size_t) g].items.front());
                            groups.erase (groups.begin() + g);
                            if (g < best) --best;
                            grew = true;
                            break;
                        }
                    }
                }
            // the heading sits just above what it names (no tall gap: fewer rack units)
            {
                auto& bg = groups[(size_t) best];
                float top = bg.area.getY();
                for (int i : bg.items) if (items[(size_t) i].kind != Kind::text) top = juce::jmin (top, boxes[(size_t) i].getY());
                const float dy = (top - 2.0f) - t.getBottom();
                if (dy > 0.0f) { items[(size_t) k].y += dy; boxes[(size_t) k] = t.translated (0.0f, dy); }
                bg.items.push_back (k);
                bg.area = bg.area.getUnion (boxes[(size_t) k]);
            }
        }
        else groups.push_back ({ t, { k } });
    }

    // each group's centre line: its control's middle (a knob's, a switch's, a meter's ...), not its
    // labels', so the controls of a row line up whatever is printed above and below them
    for (auto& grp : groups)
    {
        const auto& c = items[(size_t) grp.items.front()];
        switch (c.kind)
        {
            case Kind::knob: case Kind::selector: case Kind::polar: case Kind::rotor: case Kind::lamp: case Kind::sw:
                grp.anchor = c.y; break;
            default:
                grp.anchor = boxes[(size_t) grp.items.front()].getCentreY(); break;
        }
    }

    // reading order: the panel's own rows top to bottom (by where each group's middle is), left to right
    std::sort (groups.begin(), groups.end(), [] (const Group& a, const Group& b)
    {
        if (std::abs (a.area.getCentreY() - b.area.getCentreY()) > 0.5f * juce::jmin (a.area.getHeight(), b.area.getHeight()))
            return a.area.getCentreY() < b.area.getCentreY();
        return a.area.getCentreX() < b.area.getCentreX();
    });

    // 2. flowed across the rack in rows (the name block on the left)
    constexpr float margin = 10.0f, gapX = 12.0f, gapY = 6.0f;
    float widest = 0.0f;
    for (auto& g : groups) widest = juce::jmax (widest, g.area.getWidth());
    const float width = juce::jmax ((float) rackWidth, (float) rackTitleWidth + 2.0f * margin + widest);
    const float avail = width - (float) rackTitleWidth - 2.0f * margin;
    struct Row { std::vector<int> groups; float w = 0.0f, h = 0.0f, up = 0.0f, down = 0.0f; };
    std::vector<Row> rows (1);
    for (int g = 0; g < (int) groups.size(); ++g)
    {
        const auto& a = groups[(size_t) g].area;
        if (! rows.back().groups.empty() && rows.back().w + gapX + a.getWidth() > avail) rows.push_back ({});
        auto& r = rows.back();
        r.w = r.groups.empty() ? a.getWidth() : r.w + gapX + a.getWidth();
        const float anchor = groups[(size_t) g].anchor;
        r.up = juce::jmax (r.up, anchor - a.getY());
        r.down = juce::jmax (r.down, a.getBottom() - anchor);
        r.h = r.up + r.down;
        r.groups.push_back (g);
    }
    float contentH = 0.0f;
    for (auto& r : rows) contentH += r.h;
    contentH += gapY * (float) (rows.size() - 1);

    // 3. whole rack units: 1U is 44.45 mm of a 450.8 mm face (between the ears)
    const float unit = width * 44.45f / 450.8f;
    const float minH = presets.empty() ? 84.0f : 104.0f;           // (the name block's needs)
    const int units = juce::jmax (1, (int) std::ceil ((juce::jmax (contentH + 2.0f * margin, minH)) / unit - 0.03f));
    const float height = (float) units * unit;

    // 4. every control (and its heading) to its place: each row's controls on one centre line, the
    //    gaps between them even, across the width (a short row: no wider gaps than a full one has)
    float fullGap = gapX;
    for (auto& r : rows)
        if (r.groups.size() > 1 && avail - r.w < 0.5f * avail)
            fullGap = juce::jmax (fullGap, gapX + (avail - r.w) / (float) (r.groups.size() - 1));
    float y = (height - contentH) * 0.5f;
    for (auto& r : rows)
    {
        const float spare = avail - r.w;
        const float extra = r.groups.size() > 1 ? juce::jmin (spare / (float) (r.groups.size() - 1), juce::jmax (fullGap - gapX, 56.0f)) : 0.0f;
        float x = (float) rackTitleWidth + margin + (spare - extra * (float) (r.groups.size() - 1)) * 0.5f;
        const float line = y + r.up;
        for (int g : r.groups)
        {
            auto& grp = groups[(size_t) g];
            const float dx = x - grp.area.getX(), dy = line - grp.anchor;
            for (int k : grp.items) { items[(size_t) k].x += dx; items[(size_t) k].y += dy; }
            x += grp.area.getWidth() + gapX + extra;
        }
        y += r.h + gapY;
    }
    panelW = juce::roundToInt (width);
    panelH = juce::roundToInt (height);
}

//==============================================================================
juce::Point<float> PequodPanel::presetCentre() const
{
    return rack ? Pt ((float) rackTitleWidth / 2.0f, (float) panelH - 18.0f) : Pt ((float) panelW / 2.0f, (float) panelH - 36.0f);
}
juce::Rectangle<float> PequodPanel::presetNameRect() const { return Rect (rack ? (float) rackTitleWidth - 72.0f : 220.0f, 20.0f).withCentre (presetCentre()); }
juce::Rectangle<float> PequodPanel::presetPrevRect() const
{
    return Rect (22.0f, 20.0f).withCentre (presetCentre() - Pt (presetNameRect().getWidth() / 2.0f + 18.0f, 0.0f));
}
juce::Rectangle<float> PequodPanel::presetNextRect() const
{
    return Rect (22.0f, 20.0f).withCentre (presetCentre() + Pt (presetNameRect().getWidth() / 2.0f + 18.0f, 0.0f));
}

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
    if (! rack) say (g, "PRESET", { prev.getX() - 12.0f, prev.getCentreY() }, 1, 0, 11.0f, INK_SOFT);
    for (int k = 0; k < 2; ++k)
    {
        const auto r = k == 0 ? prev : next;
        const bool hovered = hoverButton == k + 1;
        g.setColour (hovered ? accent : displayBg());
        g.fillRoundedRectangle (r, 4.0f);
        const auto cx = r.getCentre();
        juce::Path tri;
        if (k == 1) tri.addTriangle (cx + Pt (-3.0f, -5.0f), cx + Pt (4.0f, 0.0f), cx + Pt (-3.0f, 5.0f));
        else        tri.addTriangle (cx + Pt (3.0f, -5.0f), cx + Pt (-4.0f, 0.0f), cx + Pt (3.0f, 5.0f));
        g.setColour (hovered ? onAccent() : displayInk());
        g.fillPath (tri);
    }
    g.setColour (displayBg());
    g.fillRoundedRectangle (name, 4.0f);
    g.setColour (hoverButton == 3 || presetOpen ? accent : BAKELITE_HI);
    g.drawRoundedRectangle (name.reduced (0.5f), 4.0f, 1.0f);
    const auto label = juce::isPositiveAndBelow (currentPreset, (int) presets.size())
                           ? presets[(size_t) currentPreset].name + (presetDirty ? " *" : "")
                           : juce::String (juce::CharPointer_UTF8 ("\xe2\x80\x94 choose a preset \xe2\x80\x94"));
    say (g, label, name.getCentre(), 0, 0, 12.0f, displayInk());
}

void PequodPanel::drawPresetList (juce::Graphics& g) const
{
    const auto l = presetListRect();
    g.setColour (blackAlpha (90));
    g.fillRoundedRectangle (l.translated (0.0f, 3.0f), 6.0f);
    g.setColour (displayBg());
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
        say (g, presets[(size_t) k].name, { r.getX() + 10.0f, r.getCentreY() }, -1, 0, 12.0f, hovered ? onAccent() : displayInk());
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
                if (Rect (it.x, it.y, it.w * (float) it.cols, it.rowH * (float) radioRows (it.labels.size(), it.cols)).contains (p)) return k;
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
        if (presetNameRect().contains (p))
        {
            if (rack)
            {
                // (the rack panel is short: the presets as a menu)
                juce::PopupMenu m;
                for (int k = 0; k < n; ++k) m.addItem (k + 1, presets[(size_t) k].name, true, k == currentPreset);
                juce::Component::SafePointer<PequodPanel> safe (this);
                m.showMenuAsync (juce::PopupMenu::Options().withTargetComponent (this)
                                    .withTargetScreenArea (localAreaToGlobal (presetNameRect().toNearestInt())),
                                 [safe] (int r) { if (safe != nullptr && r > 0) safe->applyPreset (r - 1); });
                return;
            }
            presetOpen = ! presetOpen; repaint(); return;
        }
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
            const int rows = radioRows (it.labels.size(), it.cols);
            const int k = juce::jlimit (0, rows - 1, (int) ((p.y - it.y) / it.rowH))
                        + rows * juce::jlimit (0, it.cols - 1, (int) ((p.x - it.x) / it.w));
            if (k != indexOf (it.p) && k < it.labels.size()) setIndex (it.p, k);
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

//==============================================================================
juce::StringArray PequodPanel::checkLayout()
{
    std::vector<std::pair<Rect, juce::String>> log;
    textLog = &log;
    {
        juce::Image img (juce::Image::ARGB, juce::jmax (1, getWidth()), juce::jmax (1, getHeight()), true);
        juce::Graphics g (img);
        paint (g);
    }
    textLog = nullptr;

    juce::StringArray problems;
    const auto bounds = getLocalBounds().toFloat();
    for (auto& [r, t] : log)
        if (! bounds.contains (r)) problems.addIfNotAlreadyThere ("OFF THE EDGE: \"" + t + "\"");
    for (size_t a = 0; a < log.size(); ++a)
        for (size_t b = a + 1; b < log.size(); ++b)
        {
            const auto i = log[a].first.getIntersection (log[b].first);
            const float small = juce::jmin (log[a].first.getWidth() * log[a].first.getHeight(), log[b].first.getWidth() * log[b].first.getHeight());
            if (! i.isEmpty() && i.getWidth() * i.getHeight() > 0.12f * small && log[a].second != log[b].second)
                problems.addIfNotAlreadyThere ("OVERLAP: \"" + log[a].second + "\" / \"" + log[b].second + "\"");
        }
    for (auto& it : items)
        if (it.kind == Kind::knob || it.kind == Kind::selector)
        {
            const float r = it.kind == Kind::selector ? 20.0f : it.r;
            const auto body = Rect (it.x - r, it.y - r, r * 2.0f, r * 2.0f).reduced (r * 0.2f);
            for (auto& [tr, t] : log)
                if (body.intersects (tr)) problems.addIfNotAlreadyThere ("ON A CONTROL (" + it.label + "): \"" + t + "\"");
        }
    return problems;
}
