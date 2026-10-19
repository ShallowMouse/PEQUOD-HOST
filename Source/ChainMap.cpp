#include "ChainMap.h"

namespace
{
    constexpr float boxH = 26.0f;
    constexpr float gapY = 10.0f;
    constexpr float pad = 6.0f;
}

ChainMap::ChainMap (HostEngine& e) : engine (e)
{
    setTooltip ("Signal chain. Click a block to jump to it in the rack; click its LED to switch it on or off.");
    engine.addChangeListener (this);
    startTimerHz (10);
}

ChainMap::~ChainMap()
{
    stopTimer();
    engine.removeChangeListener (this);
}

void ChainMap::timerCallback()
{
    const bool m = isMuted && isMuted();
    if (m != lastMuted)
    {
        lastMuted = m;
        repaint();
    }
}

void ChainMap::rebuild()
{
    blocks.clear();
    wires.clear();

    const float w = (float) getWidth();
    if (w <= 0.0f) return;

    const float x0 = pad, fullW = w - 2.0f * pad;
    float y = pad;

    auto addWire = [this] (float x, float yFrom, float yTo) { wires.push_back ({ x, yFrom, x, yTo }); };

    blocks.push_back ({ Block::Kind::input, { x0 + fullW * 0.3f, y, fullW * 0.4f, 20.0f }, 0, 0, "IN", false });
    float prevBottom = y + 20.0f;
    y = prevBottom + gapY;

    for (auto& item : engine.getItems())
    {
        if (item.type == RackItem::Type::plugin && item.plugin)
        {
            addWire (w * 0.5f, prevBottom, y);
            blocks.push_back ({ Block::Kind::plugin, { x0, y, fullW, boxH }, item.plugin->uid,
                                engine.getFlatIndex (item.plugin->uid), item.plugin->description.name, item.plugin->bypassed });
            prevBottom = y + boxH;
            y = prevBottom + gapY;
        }
        else if (item.type == RackItem::Type::split)
        {
            addWire (w * 0.5f, prevBottom, y);
            blocks.push_back ({ Block::Kind::splitHeader, { x0, y, fullW, 18.0f }, item.uid,
                                engine.getSplitNumber (item.uid), "SPLIT", false });
            const float headerBottom = y + 18.0f;
            y = headerBottom + gapY;

            const float colW = (fullW - pad) * 0.5f;
            float laneBottom[2] = { headerBottom, headerBottom };
            float maxY = y;

            for (int lane = 0; lane < 2; ++lane)
            {
                const float cx = x0 + (float) lane * (colW + pad);
                float ly = y;
                for (auto& s : item.lanes[lane])
                {
                    blocks.push_back ({ Block::Kind::plugin, { cx, ly, colW, boxH }, s->uid,
                                        engine.getFlatIndex (s->uid), s->description.name, s->bypassed });
                    ly += boxH + 4.0f;
                }
                laneBottom[lane] = ly;
                maxY = juce::jmax (maxY, ly);
            }

            // Lane wires: header -> lane top, lane bottom -> merge line
            for (int lane = 0; lane < 2; ++lane)
            {
                const float cx = x0 + (float) lane * (colW + pad) + colW * 0.5f;
                addWire (cx, headerBottom, item.lanes[lane].empty() ? maxY : y);
                if (! item.lanes[lane].empty())
                    addWire (cx, laneBottom[lane] - 4.0f, maxY);
            }
            const float leftX = x0 + colW * 0.5f, rightX = x0 + colW + pad + colW * 0.5f;
            wires.push_back ({ leftX, maxY, rightX, maxY });

            prevBottom = maxY;
            y = maxY + gapY;
        }
    }

    addWire (w * 0.5f, prevBottom, y);
    blocks.push_back ({ Block::Kind::output, { x0 + fullW * 0.3f, y, fullW * 0.4f, 20.0f }, 0, 0, "OUT", false });
    desiredHeight = (int) std::ceil (y + 20.0f + pad);

    if (getHeight() != desiredHeight && getParentComponent() != nullptr)
        setSize (getWidth(), juce::jmax (desiredHeight, getParentHeight()));
    repaint();
}

int ChainMap::blockAt (juce::Point<float> p) const
{
    for (int i = 0; i < (int) blocks.size(); ++i)
        if (blocks[(size_t) i].bounds.contains (p))
            return i;
    return -1;
}

void ChainMap::mouseMove (const juce::MouseEvent& e)
{
    auto h = blockAt (e.position);
    if (h != hover) { hover = h; repaint(); }
    setMouseCursor (h >= 0 && blocks[(size_t) h].kind != Block::Kind::input && blocks[(size_t) h].kind != Block::Kind::output
                        ? juce::MouseCursor::PointingHandCursor : juce::MouseCursor::NormalCursor);
}

void ChainMap::mouseExit (const juce::MouseEvent&)
{
    hover = -1;
    repaint();
}

void ChainMap::mouseUp (const juce::MouseEvent& e)
{
    auto i = blockAt (e.position);
    if (i < 0 || e.mouseWasDraggedSinceMouseDown()) return;
    auto b = blocks[(size_t) i];

    if (b.kind == Block::Kind::plugin && ledArea (b).contains (e.position))
    {
        auto uid = b.uid;
        auto bypass = ! b.bypassed;
        juce::MessageManager::callAsync ([this, uid, bypass] { engine.setBypassed (uid, bypass); });
        return;
    }

    if ((b.kind == Block::Kind::plugin || b.kind == Block::Kind::splitHeader) && onSelect)
        onSelect (b.uid);
}

void ChainMap::paint (juce::Graphics& g)
{
    g.fillAll (Pequod::Colours::well);

    g.setColour (Pequod::Colours::mustardLo);
    for (auto& w : wires)
        g.drawLine (w, 1.5f);

    for (int i = 0; i < (int) blocks.size(); ++i)
    {
        auto& b = blocks[(size_t) i];
        auto r = b.bounds;
        const bool hot = i == hover;

        switch (b.kind)
        {
            case Block::Kind::input:
            case Block::Kind::output:
            {
                const bool mutedOut = b.kind == Block::Kind::output && lastMuted;
                g.setColour (mutedOut ? Pequod::Colours::danger : Pequod::Colours::panelHi);
                g.fillRoundedRectangle (r, r.getHeight() * 0.5f);
                g.setColour (mutedOut ? Pequod::Colours::text : Pequod::Colours::mustard);
                g.setFont (Pequod::heading (11.0f));
                g.drawText (mutedOut ? "MUTED" : b.name, r, juce::Justification::centred);
                break;
            }

            case Block::Kind::splitHeader:
            {
                g.setColour (hot ? Pequod::Colours::panelHi.brighter (0.1f) : Pequod::Colours::panelHi);
                g.fillRoundedRectangle (r, 2.0f);
                g.setColour (Pequod::Colours::mustard);
                g.setFont (Pequod::heading (11.0f));
                g.drawText ("SPLIT " + juce::String (b.number) + "    A | B", r, juce::Justification::centred);
                break;
            }

            case Block::Kind::plugin:
            {
                g.setColour (hot ? Pequod::Colours::panelHi.brighter (0.1f) : Pequod::Colours::panel);
                g.fillRoundedRectangle (r, 2.0f);
                g.setColour (b.bypassed ? Pequod::Colours::line : Pequod::Colours::mustardLo);
                g.drawRoundedRectangle (r.reduced (0.5f), 2.0f, 1.0f);

                auto led = ledArea (b).withSizeKeepingCentre (8.0f, 8.0f);
                g.setColour (b.bypassed ? juce::Colour (0xff3a3a3a) : Pequod::Colours::mustard);
                g.fillEllipse (led);

                auto text = r.withTrimmedLeft (22.0f).reduced (2.0f, 0.0f);
                g.setFont (Pequod::heading (10.0f, false));
                g.setColour (Pequod::Colours::textDim);
                g.drawText (juce::String (b.number).paddedLeft ('0', 2), text.removeFromLeft (16.0f), juce::Justification::centredLeft);
                g.setFont (Pequod::heading (11.0f));
                g.setColour (b.bypassed ? Pequod::Colours::textDim : Pequod::Colours::text);
                g.drawText (b.name.toUpperCase(), text, juce::Justification::centredLeft, true);
                break;
            }
        }
    }
}
