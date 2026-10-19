#include "RackView.h"
#include <algorithm>
#include <map>
#include "BrowserPanel.h"
#include "NativeClip.h"

namespace
{
    constexpr int headerH = 38;
    constexpr int ear = 16;
    constexpr int laneRowH = 32;
    constexpr int gap = 8;

    bool isPluginDrag (const juce::var& d) { return d.toString().startsWith ("plugin:"); }
    bool isSlotDrag   (const juce::var& d) { return d.toString().startsWith ("slot:"); }
    bool isItemDrag   (const juce::var& d) { return d.toString().startsWith ("item:"); }
    int  dragNumber   (const juce::var& d) { return d.toString().fromFirstOccurrenceOf (":", false, false).getIntValue(); }

    void startDrag (juce::Component* source, const juce::String& id, juce::Rectangle<int> imageArea)
    {
        if (auto* c = juce::DragAndDropContainer::findParentDragContainerFor (source))
            if (! c->isDragAndDropActive())
                c->startDragging (id, source, juce::ScaledImage (source->createComponentSnapshot (imageArea)));
    }

    void styleSmallButton (juce::TextButton& b)
    {
        b.setColour (juce::TextButton::buttonColourId, Pequod::Colours::well);
    }
}

//==============================================================================
/** A top-level plugin module: header strip + (optionally) the plugin's own editor embedded below. */
class PluginModule : public juce::Component,
                     private juce::ComponentListener
{
public:
    PluginModule (RackContext& c, int slotUid, std::function<void()> relayoutFn)
        : ctx (c), uid (slotUid), relayout (std::move (relayoutFn))
    {
        addAndMakeVisible (power);
        power.onClick = [this] { ctx.engine.setBypassed (uid, ! power.getToggleState()); };
        power.setTooltip ("On / off (right-click to learn)");
        power.addMouseListener (this, false);

        for (auto* b : { &editButton, &expandButton, &removeButton })
        {
            styleSmallButton (*b);
            addAndMakeVisible (b);
        }
        editButton.setTooltip ("Open the plugin in its own window");
        expandButton.setTooltip ("Show / hide the plugin panel in the rack");
        removeButton.setTooltip ("Remove from rack");

        editButton.onClick   = [this] { setExpanded (false); ctx.openWindow (uid); };
        expandButton.onClick = [this] { setExpanded (! expanded); };
        removeButton.onClick = [this]
        {
            auto id = uid;
            auto& engine = ctx.engine;
            juce::MessageManager::callAsync ([&engine, id] { engine.removeSlot (id); });
        };

        windowButton.setButtonText ("OPEN IN WINDOW");
        styleSmallButton (windowButton);
        windowButton.onClick = [this] { ctx.openWindow (uid); };
        addChildComponent (windowButton);

        refresh();
    }

    ~PluginModule() override { dropEditor(); }

    int getUid() const { return uid; }

    void refresh()
    {
        if (auto s = ctx.engine.findSlot (uid))
        {
            name = s->description.name;
            maker = s->description.manufacturerName;
            power.setToggleState (! s->bypassed, juce::dontSendNotification);
            number = ctx.engine.getFlatIndex (uid);
        }
        expandButton.setButtonText (expanded ? "-" : "+");
        repaint();
    }

    void setExpanded (bool shouldExpand)
    {
        expanded = shouldExpand;
        note.clear();

        if (! expanded)
        {
            dropEditor();
        }
        else if (editor == nullptr)
        {
            if (ctx.isWindowOpen (uid))
                ctx.closeWindow (uid);

            if (auto s = ctx.engine.findSlot (uid))
                if (auto* inst = s->instance())
                {
                    if (inst->getActiveEditor() != nullptr)
                        note = "Plugin panel is open elsewhere";
                    else if (inst->hasEditor())
                        editor.reset (inst->createEditorAndMakeActive());
                    else
                        editor = std::make_unique<juce::GenericAudioProcessorEditor> (*inst);

                    if (editor != nullptr)
                    {
                        addAndMakeVisible (*editor);
                        editor->addComponentListener (this);
                    }
                    else if (note.isEmpty())
                        note = "This plugin has no panel";
                }
        }

        refresh();
        relayout();
    }

    bool isExpanded() const { return expanded; }

    void dropEditor()
    {
        if (editor != nullptr)
        {
            editor->removeComponentListener (this);
            editor.reset();
        }
    }

    int getDesiredHeight (int width)
    {
        if (! expanded) return headerH;

        if (editor != nullptr)
        {
            if (editor->getWidth() > width - 2 * ear - 8)
            {
                // Too wide to sit in the rack: offer a window instead.
                dropEditor();
                note = "Plugin panel is wider than the rack";
            }
            else
                return headerH + editor->getHeight() + 12;
        }
        return headerH + (note.isNotEmpty() ? 44 : 0);
    }

    void paint (juce::Graphics& g) override
    {
        auto active = power.getToggleState();
        Pequod::drawModulePanel (g, getLocalBounds().toFloat(), active);

        auto h = getLocalBounds().withHeight (headerH).reduced (ear, 0);
        h.removeFromLeft (36); // power
        auto nameArea = h.withRight (editButton.getX() - 8);

        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::heading (11.0f, false));
        auto num = nameArea.removeFromLeft (26);
        g.drawText (juce::String (number).paddedLeft ('0', 2), num, juce::Justification::centredLeft);

        g.setColour (active ? Pequod::Colours::text : Pequod::Colours::textDim);
        g.setFont (Pequod::heading (16.0f));
        auto nameW = juce::jmin (nameArea.getWidth(), juce::GlyphArrangement::getStringWidthInt (Pequod::heading (16.0f), name.toUpperCase()) + 4);
        g.drawText (name.toUpperCase(), nameArea.removeFromLeft (nameW), juce::Justification::centredLeft, true);

        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::body (12.0f));
        g.drawText ("  " + maker, nameArea, juce::Justification::centredLeft, true);

        if (expanded)
        {
            auto body = getLocalBounds().withTrimmedTop (headerH).reduced (ear + 4, 4);
            g.setColour (Pequod::Colours::well);
            g.fillRect (body);

            if (note.isNotEmpty() && editor == nullptr)
            {
                g.setColour (Pequod::Colours::textDim);
                g.setFont (Pequod::body (13.0f));
                g.drawText (note, body.withTrimmedRight (170).reduced (10, 0), juce::Justification::centredLeft);
            }
        }
    }

    void resized() override
    {
        auto h = getLocalBounds().withHeight (headerH).reduced (ear + 4, 7);
        power.setBounds (h.removeFromLeft (28));
        removeButton.setBounds (h.removeFromRight (26));
        h.removeFromRight (4);
        expandButton.setBounds (h.removeFromRight (26));
        h.removeFromRight (4);
        editButton.setBounds (h.removeFromRight (48));

        auto body = getLocalBounds().withTrimmedTop (headerH).reduced (ear + 4, 4);
        if (editor != nullptr)
            editor->setTopLeftPosition (body.getCentreX() - editor->getWidth() / 2, body.getY() + 2);

        windowButton.setVisible (expanded && editor == nullptr && note.isNotEmpty());
        windowButton.setBounds (body.removeFromRight (160).reduced (8, 8));
    }

    void mouseDown (const juce::MouseEvent& e) override
    {
        if (! e.mods.isPopupMenu()) return;

        juce::PopupMenu m;
        m.addItem ("MIDI / OSC learn: on-off", [this] { ctx.control.armLearnFor (Mapping::Target::bypass, uid); });
        m.addItem ("MIDI / OSC learn: a parameter...", [this]
        {
            ctx.control.armLearn();
            ctx.status ("Move a control on the plugin panel, then the MIDI / OSC control");
        });
        m.addSeparator();
        m.addItem ("Open in window", [this] { setExpanded (false); ctx.openWindow (uid); });
        m.addItem ("Remove", [this] { removeButton.triggerClick(); });
        m.showMenuAsync (juce::PopupMenu::Options());
    }

    void mouseDrag (const juce::MouseEvent& e) override
    {
        if (e.eventComponent == this && e.mouseDownPosition.y < headerH && e.getDistanceFromDragStart() > 6)
            startDrag (this, "slot:" + juce::String (uid), getLocalBounds().withHeight (headerH));
    }

private:
    RackContext& ctx;
    int uid;
    std::function<void()> relayout;

    PowerButton power;
    juce::TextButton editButton { "EDIT" }, expandButton { "-" }, removeButton { "X" }, windowButton;
    std::unique_ptr<juce::AudioProcessorEditor> editor;
    bool expanded = true;
    juce::String name, maker, note;
    int number = 0;

    void componentMovedOrResized (juce::Component&, bool, bool wasResized) override
    {
        if (wasResized) relayout();
    }
};

//==============================================================================
/** Compact row for a plugin inside a split lane. */
class LaneSlotRow : public juce::Component
{
public:
    LaneSlotRow (RackContext& c, int slotUid) : ctx (c), uid (slotUid)
    {
        addAndMakeVisible (power);
        power.onClick = [this] { ctx.engine.setBypassed (uid, ! power.getToggleState()); };
        power.addMouseListener (this, false);

        for (auto* b : { &editButton, &removeButton })
        {
            styleSmallButton (*b);
            addAndMakeVisible (b);
        }
        editButton.onClick = [this] { ctx.openWindow (uid); };
        removeButton.onClick = [this]
        {
            auto id = uid;
            auto& engine = ctx.engine;
            juce::MessageManager::callAsync ([&engine, id] { engine.removeSlot (id); });
        };

        if (auto s = ctx.engine.findSlot (uid))
        {
            name = s->description.name;
            power.setToggleState (! s->bypassed, juce::dontSendNotification);
            number = ctx.engine.getFlatIndex (uid);
        }
    }

    int getUid() const { return uid; }

    void paint (juce::Graphics& g) override
    {
        auto r = getLocalBounds().toFloat().reduced (0, 2);
        g.setColour (Pequod::Colours::panelHi);
        g.fillRoundedRectangle (r, 2.0f);
        g.setColour (power.getToggleState() ? Pequod::Colours::mustardLo : Pequod::Colours::line);
        g.drawRoundedRectangle (r, 2.0f, 1.0f);

        auto t = getLocalBounds().withTrimmedLeft (30).withRight (editButton.getX() - 4);
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::heading (10.0f, false));
        g.drawText (juce::String (number).paddedLeft ('0', 2), t.removeFromLeft (20), juce::Justification::centredLeft);
        g.setColour (power.getToggleState() ? Pequod::Colours::text : Pequod::Colours::textDim);
        g.setFont (Pequod::heading (13.0f));
        g.drawText (name.toUpperCase(), t, juce::Justification::centredLeft, true);
    }

    void resized() override
    {
        auto r = getLocalBounds().reduced (4, 5);
        power.setBounds (r.removeFromLeft (22));
        removeButton.setBounds (r.removeFromRight (22));
        r.removeFromRight (3);
        editButton.setBounds (r.removeFromRight (40));
    }

    void mouseDown (const juce::MouseEvent& e) override
    {
        if (! e.mods.isPopupMenu()) return;
        juce::PopupMenu m;
        m.addItem ("MIDI / OSC learn: on-off", [this] { ctx.control.armLearnFor (Mapping::Target::bypass, uid); });
        m.addItem ("Open in window", [this] { ctx.openWindow (uid); });
        m.addItem ("Remove", [this] { removeButton.triggerClick(); });
        m.showMenuAsync (juce::PopupMenu::Options());
    }

    void mouseDrag (const juce::MouseEvent& e) override
    {
        if (e.eventComponent == this && e.getDistanceFromDragStart() > 6)
            startDrag (this, "slot:" + juce::String (uid), getLocalBounds());
    }

    void mouseDoubleClick (const juce::MouseEvent&) override { ctx.openWindow (uid); }

private:
    RackContext& ctx;
    int uid;
    PowerButton power;
    juce::TextButton editButton { "EDIT" }, removeButton { "X" };
    juce::String name;
    int number = 0;
};

//==============================================================================
/** One lane (A or B) of a split: a drop target holding compact rows. */
class LaneComponent : public juce::Component,
                      public juce::DragAndDropTarget
{
public:
    LaneComponent (RackContext& c, int splitItemUid, int laneIndex)
        : ctx (c), splitUid (splitItemUid), lane (laneIndex) {}

    void rebuild (const std::vector<std::shared_ptr<PluginSlot>>& slots)
    {
        rows.clear();
        for (auto& s : slots)
        {
            rows.push_back (std::make_unique<LaneSlotRow> (ctx, s->uid));
            addAndMakeVisible (*rows.back());
        }
        resized();
    }

    int getDesiredHeight() const { return 22 + (int) rows.size() * laneRowH + 34; }

    void paint (juce::Graphics& g) override
    {
        auto r = getLocalBounds();
        g.setColour (Pequod::Colours::well);
        g.fillRect (r);

        auto title = r.removeFromTop (22);
        g.setColour (Pequod::Colours::mustard);
        g.setFont (Pequod::heading (13.0f));
        g.drawText (lane == 0 ? "A" : "B", title.reduced (8, 0), juce::Justification::centredLeft);

        auto drop = getLocalBounds().removeFromBottom (34).reduced (6, 5);
        g.setColour (dragOver ? Pequod::Colours::mustard : Pequod::Colours::line);
        g.drawRoundedRectangle (drop.toFloat(), 2.0f, 1.0f);
        g.setFont (Pequod::heading (11.0f, false));
        g.setColour (Pequod::Colours::textDim);
        g.drawText ("+ DROP PLUGIN", drop, juce::Justification::centred);
    }

    void paintOverChildren (juce::Graphics& g) override
    {
        if (dragOver && insertIndex >= 0)
        {
            g.setColour (Pequod::Colours::mustard);
            g.fillRect (4, 22 + insertIndex * laneRowH - 1, getWidth() - 8, 3);
        }
    }

    void resized() override
    {
        auto r = getLocalBounds().withTrimmedTop (22).reduced (6, 0);
        for (auto& row : rows)
            row->setBounds (r.removeFromTop (laneRowH));
    }

    bool isInterestedInDragSource (const SourceDetails& d) override
    {
        return isPluginDrag (d.description) || isSlotDrag (d.description);
    }

    void itemDragEnter (const SourceDetails& d) override { itemDragMove (d); }
    void itemDragMove (const SourceDetails& d) override
    {
        dragOver = true;
        insertIndex = juce::jlimit (0, (int) rows.size(), (d.localPosition.y - 22 + laneRowH / 2) / laneRowH);
        repaint();
    }
    void itemDragExit (const SourceDetails&) override { dragOver = false; repaint(); }

    void itemDropped (const SourceDetails& d) override
    {
        dragOver = false;
        repaint();

        int itemIndex = -1;
        auto& items = ctx.engine.getItems();
        for (int i = 0; i < (int) items.size(); ++i)
            if (items[(size_t) i].uid == splitUid) itemIndex = i;
        if (itemIndex < 0) return;

        RackLocation where { itemIndex, lane, insertIndex };
        auto desc = d.description;
        auto& ctxRef = ctx;

        juce::MessageManager::callAsync ([&ctxRef, desc, where]
        {
            if (isPluginDrag (desc))
            {
                auto types = ctxRef.engine.knownPlugins.getTypes();
                auto idx = dragNumber (desc);
                if (juce::isPositiveAndBelow (idx, types.size()))
                {
                    juce::String err;
                    if (ctxRef.engine.addPlugin (types.getReference (idx), where, err) == 0)
                        ctxRef.status ("Couldn't load plugin: " + err);
                }
            }
            else if (isSlotDrag (desc))
            {
                ctxRef.engine.moveSlot (dragNumber (desc), where);
            }
        });
    }

private:
    RackContext& ctx;
    int splitUid, lane;
    std::vector<std::unique_ptr<LaneSlotRow>> rows;
    bool dragOver = false;
    int insertIndex = -1;
};

//==============================================================================
/** Split module: header with A/B crossfader, two lanes side by side. */
class SplitModule : public juce::Component
{
public:
    SplitModule (RackContext& c, int itemUid)
        : ctx (c), uid (itemUid), laneA (c, itemUid, 0), laneB (c, itemUid, 1)
    {
        addAndMakeVisible (laneA);
        addAndMakeVisible (laneB);

        mix.setSliderStyle (juce::Slider::LinearHorizontal);
        mix.setTextBoxStyle (juce::Slider::NoTextBox, false, 0, 0);
        mix.setRange (0.0, 1.0, 0.001);
        mix.setDoubleClickReturnValue (true, 0.5);
        mix.setTooltip ("A / B balance - centre plays both (right-click to learn)");
        mix.onValueChange = [this] { ctx.engine.setSplitMix (uid, (float) mix.getValue()); };
        mix.onLearn = [this] { ctx.control.armLearnFor (Mapping::Target::splitMix, uid); };
        addAndMakeVisible (mix);

        styleSmallButton (removeButton);
        removeButton.setTooltip ("Remove split and the plugins inside it");
        removeButton.onClick = [this]
        {
            auto id = uid;
            auto& engine = ctx.engine;
            juce::MessageManager::callAsync ([&engine, id] { engine.removeItem (id); });
        };
        addAndMakeVisible (removeButton);
    }

    int getUid() const { return uid; }

    void rebuild (const RackItem& item)
    {
        laneA.rebuild (item.lanes[0]);
        laneB.rebuild (item.lanes[1]);
        syncMix (item.mix);
        number = ctx.engine.getSplitNumber (uid);
        repaint();
    }

    void syncMix (float v)
    {
        if (! mix.isMouseButtonDown() && std::abs (mix.getValue() - v) > 0.0005)
            mix.setValue (v, juce::dontSendNotification);
    }

    int getDesiredHeight() const
    {
        return headerH + 6 + juce::jmax (laneA.getDesiredHeight(), laneB.getDesiredHeight()) + 10;
    }

    void paint (juce::Graphics& g) override
    {
        Pequod::drawModulePanel (g, getLocalBounds().toFloat(), true);

        auto h = getLocalBounds().withHeight (headerH).reduced (ear + 10, 0);
        g.setColour (Pequod::Colours::mustard);
        g.setFont (Pequod::heading (16.0f));
        g.drawText ("SPLIT " + juce::String (number), h.removeFromLeft (110), juce::Justification::centredLeft);

        g.setFont (Pequod::heading (12.0f));
        g.setColour (Pequod::Colours::textDim);
        g.drawText ("A", mix.getBounds().translated (-18, 0).withWidth (14), juce::Justification::centred);
        g.drawText ("B", mix.getBounds().translated (mix.getWidth() + 4, 0).withWidth (14), juce::Justification::centred);
    }

    void resized() override
    {
        auto h = getLocalBounds().withHeight (headerH).reduced (ear + 6, 8);
        removeButton.setBounds (h.removeFromRight (26));
        h.removeFromLeft (130);
        h.removeFromRight (40);
        mix.setBounds (h.withSizeKeepingCentre (juce::jmin (260, h.getWidth()), h.getHeight()));

        auto body = getLocalBounds().withTrimmedTop (headerH + 6).reduced (ear + 6, 0).withTrimmedBottom (10);
        auto half = (body.getWidth() - 8) / 2;
        laneA.setBounds (body.removeFromLeft (half));
        body.removeFromLeft (8);
        laneB.setBounds (body);
    }

    void mouseDrag (const juce::MouseEvent& e) override
    {
        if (e.eventComponent == this && e.mouseDownPosition.y < headerH && e.getDistanceFromDragStart() > 6)
            startDrag (this, "item:" + juce::String (uid), getLocalBounds().withHeight (headerH));
    }

private:
    RackContext& ctx;
    int uid;
    int number = 0;
    LaneComponent laneA, laneB;
    LearnableSlider mix;
    juce::TextButton removeButton { "X" };
};

//==============================================================================
class RackContent : public juce::Component,
                    public juce::DragAndDropTarget,
                    private juce::ChangeListener,
                    private juce::AsyncUpdater,
                    private juce::Timer
{
public:
    explicit RackContent (RackContext& c) : ctx (c)
    {
        addSplitButton.setButtonText ("+ SPLIT");
        addSplitButton.setTooltip ("Add an A/B split module - drop plugins into each lane");
        styleSmallButton (addSplitButton);
        addSplitButton.onClick = [this] { ctx.engine.addSplit (-1); };
        addAndMakeVisible (addSplitButton);

        ctx.engine.addChangeListener (this);
        rebuild();
        startTimerHz (15);
    }

    ~RackContent() override { ctx.engine.removeChangeListener (this); }

    void dropEditor (int uid)
    {
        auto it = pluginModules.find (uid);
        if (it != pluginModules.end())
            it->second->dropEditor();
    }

    void setAllExpanded (bool e)
    {
        for (auto& [uid, m] : pluginModules)
            m->setExpanded (e);
    }

    void rebuild()
    {
        auto& items = ctx.engine.getItems();
        std::map<int, std::unique_ptr<PluginModule>> keepPlugins;
        std::map<int, std::unique_ptr<SplitModule>> keepSplits;
        order.clear();

        for (auto& item : items)
        {
            if (item.type == RackItem::Type::plugin && item.plugin)
            {
                auto id = item.plugin->uid;
                auto it = pluginModules.find (id);
                std::unique_ptr<PluginModule> m;
                if (it != pluginModules.end()) { m = std::move (it->second); pluginModules.erase (it); }
                else
                {
                    m = std::make_unique<PluginModule> (ctx, id, [this] { triggerAsyncUpdate(); });
                    addAndMakeVisible (*m);
                    m->setExpanded (true);
                }
                m->refresh();
                order.push_back (m.get());
                keepPlugins[id] = std::move (m);
            }
            else if (item.type == RackItem::Type::split)
            {
                auto it = splitModules.find (item.uid);
                std::unique_ptr<SplitModule> m;
                if (it != splitModules.end()) { m = std::move (it->second); splitModules.erase (it); }
                else
                {
                    m = std::make_unique<SplitModule> (ctx, item.uid);
                    addAndMakeVisible (*m);
                }
                m->rebuild (item);
                order.push_back (m.get());
                keepSplits[item.uid] = std::move (m);
            }
        }

        pluginModules = std::move (keepPlugins); // old ones (removed / moved into lanes) are destroyed here
        splitModules = std::move (keepSplits);
        layout();
    }

    void layout()
    {
        auto w = getWidth();
        if (w <= 0) return;

        int y = gap;
        for (auto* c : order)
        {
            int h = headerH;
            if (auto* pm = dynamic_cast<PluginModule*> (c)) h = pm->getDesiredHeight (w - 2 * gap);
            if (auto* sm = dynamic_cast<SplitModule*> (c))  h = sm->getDesiredHeight();
            c->setBounds (gap, y, w - 2 * gap, h);
            y += h + gap;
        }

        dropZone = juce::Rectangle<int> (gap, y, w - 2 * gap, order.empty() ? 160 : 60);
        y = dropZone.getBottom() + gap;
        addSplitButton.setBounds (juce::Rectangle<int> (gap + ear, y, 110, 28));
        y += 28 + gap * 3;

        auto minH = getParentComponent() != nullptr ? getParentComponent()->getHeight() : 0;
        if (getHeight() != juce::jmax (y, minH))
            setSize (w, juce::jmax (y, minH));
        repaint();

        if (onLayoutChanged) onLayoutChanged();
    }

    std::function<void()> onLayoutChanged;

    /** Bounds (in this component) of the module showing a slot or split; empty if not found. */
    juce::Rectangle<int> getModuleBoundsFor (int uid) const
    {
        if (auto it = pluginModules.find (uid); it != pluginModules.end()) return it->second->getBounds();
        if (auto it = splitModules.find (uid); it != splitModules.end())   return it->second->getBounds();

        RackLocation loc;
        if (ctx.engine.locateSlot (uid, loc) && loc.lane >= 0)
        {
            auto& items = ctx.engine.getItems();
            if (juce::isPositiveAndBelow (loc.itemIndex, (int) items.size()))
                if (auto it = splitModules.find (items[(size_t) loc.itemIndex].uid); it != splitModules.end())
                    return it->second->getBounds();
        }
        return {};
    }

    void flash (juce::Rectangle<int> area)
    {
        flashArea = area;
        flashUntil = juce::Time::getMillisecondCounterHiRes() + 900.0;
        repaint();
    }

    void paint (juce::Graphics& g) override
    {
        g.fillAll (Pequod::Colours::rail);

        // Rack rails with mounting holes, Guitar Rig style.
        for (int x : { 2, getWidth() - 8 })
        {
            g.setColour (juce::Colour (0xff1d1d1d));
            g.fillRect (x, 0, 6, getHeight());
            g.setColour (juce::Colour (0xff0a0a0a));
            for (int y = 8; y < getHeight(); y += 22)
                g.fillRect (x + 1, y, 4, 8);
        }

        g.setColour (dragOver && insertIndex == (int) order.size() ? Pequod::Colours::mustard : Pequod::Colours::line);
        g.drawRoundedRectangle (dropZone.toFloat().reduced (ear, 4), 3.0f, 1.0f);
        g.setColour (Pequod::Colours::textDim);
        g.setFont (Pequod::heading (order.empty() ? 18.0f : 12.0f));
        g.drawText (order.empty() ? "DRAG PLUGINS HERE" : "+ DROP PLUGIN", dropZone, juce::Justification::centred);
        if (order.empty())
        {
            g.setFont (Pequod::body (13.0f));
            g.drawText ("from the browser on the left, or double-click one",
                        dropZone.withTrimmedTop (60), juce::Justification::centred);
        }
    }

    void paintOverChildren (juce::Graphics& g) override
    {
        if (! flashArea.isEmpty())
        {
            g.setColour (Pequod::Colours::mustard);
            g.drawRoundedRectangle (flashArea.toFloat().expanded (2.0f), 4.0f, 2.5f);
        }

        if (! dragOver || insertIndex < 0) return;
        int y = insertIndex < (int) order.size() ? order[(size_t) insertIndex]->getY() - gap / 2
                                                 : dropZone.getY() - gap / 2;
        g.setColour (Pequod::Colours::mustard);
        g.fillRect (gap, y - 1, getWidth() - 2 * gap, 3);
    }

    //== Drag and drop (top level)
    bool isInterestedInDragSource (const SourceDetails& d) override
    {
        return isPluginDrag (d.description) || isSlotDrag (d.description) || isItemDrag (d.description);
    }

    void itemDragEnter (const SourceDetails& d) override { itemDragMove (d); }
    void itemDragMove (const SourceDetails& d) override
    {
        dragOver = true;
        insertIndex = (int) order.size();
        for (int i = 0; i < (int) order.size(); ++i)
            if (d.localPosition.y < order[(size_t) i]->getBounds().getCentreY())
            {
                insertIndex = i;
                break;
            }
        repaint();
    }
    void itemDragExit (const SourceDetails&) override { dragOver = false; repaint(); }

    void itemDropped (const SourceDetails& d) override
    {
        dragOver = false;
        repaint();
        auto desc = d.description;
        auto index = insertIndex;
        auto& ctxRef = ctx;

        juce::MessageManager::callAsync ([&ctxRef, desc, index]
        {
            if (isPluginDrag (desc))
            {
                auto types = ctxRef.engine.knownPlugins.getTypes();
                auto idx = dragNumber (desc);
                if (juce::isPositiveAndBelow (idx, types.size()))
                {
                    juce::String err;
                    if (ctxRef.engine.addPlugin (types.getReference (idx), { index, -1, 0 }, err) == 0)
                        ctxRef.status ("Couldn't load plugin: " + err);
                }
            }
            else if (isSlotDrag (desc)) ctxRef.engine.moveSlot (dragNumber (desc), { index, -1, 0 });
            else if (isItemDrag (desc)) ctxRef.engine.moveItem (dragNumber (desc), index);
        });
    }

private:
    RackContext& ctx;
    std::map<int, std::unique_ptr<PluginModule>> pluginModules;
    std::map<int, std::unique_ptr<SplitModule>> splitModules;
    std::vector<juce::Component*> order;
    juce::TextButton addSplitButton;
    juce::Rectangle<int> dropZone;
    bool dragOver = false;
    int insertIndex = -1;
    juce::Rectangle<int> flashArea;
    double flashUntil = 0;

    void changeListenerCallback (juce::ChangeBroadcaster*) override { rebuild(); }
    void handleAsyncUpdate() override { layout(); }
    void resized() override { layout(); }

    void timerCallback() override
    {
        if (! flashArea.isEmpty() && juce::Time::getMillisecondCounterHiRes() > flashUntil)
        {
            flashArea = {};
            repaint();
        }

        auto& items = ctx.engine.getItems();
        for (auto& item : items)
            if (item.type == RackItem::Type::split)
            {
                auto it = splitModules.find (item.uid);
                if (it != splitModules.end())
                    it->second->syncMix (item.mix);
            }
    }
};

//==============================================================================
RackView::RackView (RackContext c) : ctx (std::move (c))
{
    content = std::make_unique<RackContent> (ctx);
    content->onLayoutChanged = [this] { updateNativeClipping(); };
    viewport.onVisibleAreaChanged = [this] { updateNativeClipping(); };
    viewport.setViewedComponent (content.get(), false);
    viewport.setScrollBarsShown (true, false);
    viewport.setScrollBarThickness (10);
    addAndMakeVisible (viewport);
    startTimerHz (10);
}

RackView::~RackView()
{
    stopTimer();
    viewport.setViewedComponent (nullptr, false);
    content.reset();
}

void RackView::dropEditor (int uid) { content->dropEditor (uid); }
void RackView::collapseAll()        { content->setAllExpanded (false); }
void RackView::expandAll()          { content->setAllExpanded (true); }

void RackView::paint (juce::Graphics& g) { g.fillAll (Pequod::Colours::rail); }

void RackView::updateNativeClipping()
{
    if (content != nullptr && isShowing())
        NativeClip::clipToArea (*content, viewport.getScreenBounds().withTrimmedRight (viewport.getScrollBarThickness()));
}

void RackView::showModule (int uid)
{
    auto b = content->getModuleBoundsFor (uid);
    if (b.isEmpty()) return;
    viewport.setViewPosition (0, juce::jmax (0, b.getY() - 8));
    content->flash (b);
}

void RackView::resized()
{
    viewport.setBounds (getLocalBounds());
    content->setSize (viewport.getWidth() - viewport.getScrollBarThickness(), content->getHeight());
    content->layout();
}
