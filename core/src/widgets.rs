use std::ffi::c_void;

use crate::ue::{self, Anchors, ChildSize, Function, LinearColor, Margin, Name, Obj, SlateColor, Text, Vec2};

pub const VISIBLE: u8 = 0;
pub const COLLAPSED: u8 = 1;
pub const HIDDEN: u8 = 2;
pub const HIT_TEST_INVISIBLE: u8 = 3;
pub const SELF_HIT_TEST_INVISIBLE: u8 = 4;

pub const FILL: u8 = 0;
pub const START: u8 = 1;
pub const CENTER: u8 = 2;
pub const END: u8 = 3;

const SIZE_AUTO: u8 = 0;
const SIZE_FILL: u8 = 1;
const JUSTIFY_CENTER: u8 = 1;
const CLIP_TO_BOUNDS: u8 = 1;
const DO_NOT_LOCK_MOUSE: u8 = 0;

pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> LinearColor {
    LinearColor { r, g, b, a }
}

/// How a child sits in a horizontal or vertical box.
#[derive(Clone, Copy)]
pub struct Place {
    pub fill: bool,
    pub padding: Margin,
    pub horizontal: u8,
    pub vertical: u8,
}

impl Place {
    pub fn auto() -> Self {
        Self { fill: false, padding: Margin::default(), horizontal: FILL, vertical: CENTER }
    }

    pub fn fill() -> Self {
        Self { fill: true, ..Self::auto() }
    }

    pub fn pad(mut self, padding: Margin) -> Self {
        self.padding = padding;
        self
    }

    pub fn align(mut self, horizontal: u8, vertical: u8) -> Self {
        self.horizontal = horizontal;
        self.vertical = vertical;
        self
    }
}

struct BoxSlot {
    size: Function,
    padding: Function,
    horizontal: Function,
    vertical: Function,
}

impl BoxSlot {
    fn resolve(class: Obj) -> Result<Self, String> {
        Ok(Self {
            size: Function::resolve(class, "SetSize", &["InSize"])?,
            padding: Function::resolve(class, "SetPadding", &["InPadding"])?,
            horizontal: Function::resolve(class, "SetHorizontalAlignment", &["InHorizontalAlignment"])?,
            vertical: Function::resolve(class, "SetVerticalAlignment", &["InVerticalAlignment"])?,
        })
    }

    fn apply(&mut self, slot: Obj, place: Place) {
        let size = ChildSize { value: 1.0, rule: if place.fill { SIZE_FILL } else { SIZE_AUTO } };
        self.size.call(slot, |a| a.set(0, size));
        self.padding.call(slot, |a| a.set(0, place.padding));
        self.horizontal.call(slot, |a| a.set(0, place.horizontal));
        self.vertical.call(slot, |a| a.set(0, place.vertical));
    }
}

/// UMG building blocks for windows made from plain engine widgets, polled rather than bound to events.
pub struct Widgets {
    blueprint_library: Obj,
    layout_library: Obj,
    text_library: Obj,
    pub user_widget_class: Obj,
    pub canvas_class: Obj,
    pub border_class: Obj,
    pub button_class: Obj,
    pub text_class: Obj,
    pub slider_class: Obj,
    pub size_box_class: Obj,
    pub vbox_class: Obj,
    pub hbox_class: Obj,
    pub scroll_class: Obj,
    pub image_class: Obj,
    /// Scales its child to fit, keeping its shape (ScaleToFit is the default).
    pub scale_box_class: Obj,
    widget_tree_offset: usize,
    root_widget_offset: usize,
    font_offset: usize,
    font_size_offset: usize,
    typeface_offset: usize,
    bold: Name,
    create: Function,
    add_to_viewport: Function,
    remove_from_parent: Function,
    set_visibility: Function,
    set_render_opacity: Function,
    set_clipping: Function,
    set_angle: Function,
    set_pivot: Function,
    image_material: Function,
    image_dynamic_material: Function,
    texture_parameter: Function,
    rendering: Obj,
    import: Function,
    add_child: Function,
    canvas_position: Function,
    canvas_size: Function,
    canvas_alignment: Function,
    canvas_anchors: Function,
    canvas_offsets: Function,
    canvas_auto_size: Function,
    canvas_z_order: Function,
    hbox_slot: BoxSlot,
    vbox_slot: BoxSlot,
    border_color: Function,
    border_padding: Function,
    button_color: Function,
    button_pressed: Function,
    text_set: Function,
    text_color: Function,
    text_justify: Function,
    string_to_text: Function,
    width_override: Function,
    height_override: Function,
    slider_min: Function,
    slider_max: Function,
    slider_set: Function,
    slider_get: Function,
    slider_step: Function,
    slider_bar_color: Function,
    slider_handle_color: Function,
    image_texture: Function,
    image_color: Function,
    input_ui_only: Function,
    input_game_only: Function,
    mouse_position: Function,
    viewport_size: Function,
    viewport_scale: Function,
}

impl Widgets {
    pub fn resolve() -> Result<Self, String> {
        let widget = ue::find("/Script/UMG.Widget")?;
        let panel = ue::find("/Script/UMG.PanelWidget")?;
        let user_widget_class = ue::find("/Script/UMG.UserWidget")?;
        let canvas_class = ue::find("/Script/UMG.CanvasPanel")?;
        let canvas_slot = ue::find("/Script/UMG.CanvasPanelSlot")?;
        let border_class = ue::find("/Script/UMG.Border")?;
        let button_class = ue::find("/Script/UMG.Button")?;
        let text_class = ue::find("/Script/UMG.TextBlock")?;
        let slider_class = ue::find("/Script/UMG.Slider")?;
        let size_box_class = ue::find("/Script/UMG.SizeBox")?;
        let vbox_class = ue::find("/Script/UMG.VerticalBox")?;
        let hbox_class = ue::find("/Script/UMG.HorizontalBox")?;
        let scroll_class = ue::find("/Script/UMG.ScrollBox")?;
        let image_class = ue::find("/Script/UMG.Image")?;
        let scale_box_class = ue::find("/Script/UMG.ScaleBox")?;
        let tree_class = ue::find("/Script/UMG.WidgetTree")?;
        let text_layout = ue::find("/Script/UMG.TextLayoutWidget")?;
        let font_info = ue::find("/Script/SlateCore.SlateFontInfo")?;
        let blueprint_library = ue::find("/Script/UMG.Default__WidgetBlueprintLibrary")?;
        let layout_library = ue::find("/Script/UMG.Default__WidgetLayoutLibrary")?;
        let text_library = ue::find("/Script/Engine.Default__KismetTextLibrary")?;
        let rendering = ue::find("/Script/Engine.Default__KismetRenderingLibrary")?;
        Ok(Self {
            widget_tree_offset: ue::property_offset(user_widget_class, "WidgetTree")?,
            root_widget_offset: ue::property_offset(tree_class, "RootWidget")?,
            font_offset: ue::property_offset(text_class, "Font")?,
            font_size_offset: ue::property_offset(font_info, "Size")?,
            typeface_offset: ue::property_offset(font_info, "TypefaceFontName")?,
            bold: Name::new("Bold"),
            create: Function::resolve(blueprint_library, "Create", &["WorldContextObject", "WidgetType", "OwningPlayer"])?,
            add_to_viewport: Function::resolve(user_widget_class, "AddToViewport", &["ZOrder"])?,
            remove_from_parent: Function::resolve(widget, "RemoveFromParent", &[])?,
            set_visibility: Function::resolve(widget, "SetVisibility", &["InVisibility"])?,
            set_render_opacity: Function::resolve(widget, "SetRenderOpacity", &["InOpacity"])?,
            set_clipping: Function::resolve(widget, "SetClipping", &["InClipping"])?,
            set_angle: Function::resolve(widget, "SetRenderTransformAngle", &["Angle"])?,
            set_pivot: Function::resolve(widget, "SetRenderTransformPivot", &["Pivot"])?,
            image_material: Function::resolve(image_class, "SetBrushFromMaterial", &["Material"])?,
            image_dynamic_material: Function::resolve(image_class, "GetDynamicMaterial", &[])?,
            texture_parameter: Function::resolve(
                ue::find("/Script/Engine.MaterialInstanceDynamic")?,
                "SetTextureParameterValue",
                &["ParameterName", "Value"],
            )?,
            import: Function::resolve(rendering, "ImportBufferAsTexture2D", &["WorldContextObject", "Buffer"])?,
            rendering,
            add_child: Function::resolve(panel, "AddChild", &["Content"])?,
            canvas_position: Function::resolve(canvas_slot, "SetPosition", &["InPosition"])?,
            canvas_size: Function::resolve(canvas_slot, "SetSize", &["InSize"])?,
            canvas_alignment: Function::resolve(canvas_slot, "SetAlignment", &["InAlignment"])?,
            canvas_anchors: Function::resolve(canvas_slot, "SetAnchors", &["InAnchors"])?,
            canvas_offsets: Function::resolve(canvas_slot, "SetOffsets", &["InOffset"])?,
            canvas_auto_size: Function::resolve(canvas_slot, "SetAutoSize", &["InbAutoSize"])?,
            canvas_z_order: Function::resolve(canvas_slot, "SetZOrder", &["InZOrder"])?,
            hbox_slot: BoxSlot::resolve(ue::find("/Script/UMG.HorizontalBoxSlot")?)?,
            vbox_slot: BoxSlot::resolve(ue::find("/Script/UMG.VerticalBoxSlot")?)?,
            border_color: Function::resolve(border_class, "SetBrushColor", &["InBrushColor"])?,
            border_padding: Function::resolve(border_class, "SetPadding", &["InPadding"])?,
            button_color: Function::resolve(button_class, "SetBackgroundColor", &["InBackgroundColor"])?,
            button_pressed: Function::resolve(button_class, "IsPressed", &[])?,
            text_set: Function::resolve(text_class, "SetText", &["InText"])?,
            text_color: Function::resolve(text_class, "SetColorAndOpacity", &["InColorAndOpacity"])?,
            text_justify: Function::resolve(text_layout, "SetJustification", &["InJustification"])?,
            string_to_text: Function::resolve(text_library, "Conv_StringToText", &["InString"])?,
            width_override: Function::resolve(size_box_class, "SetWidthOverride", &["InWidthOverride"])?,
            height_override: Function::resolve(size_box_class, "SetHeightOverride", &["InHeightOverride"])?,
            slider_min: Function::resolve(slider_class, "SetMinValue", &["InValue"])?,
            slider_max: Function::resolve(slider_class, "SetMaxValue", &["InValue"])?,
            slider_set: Function::resolve(slider_class, "SetValue", &["InValue"])?,
            slider_get: Function::resolve(slider_class, "GetValue", &[])?,
            slider_step: Function::resolve(slider_class, "SetStepSize", &["InValue"])?,
            slider_bar_color: Function::resolve(slider_class, "SetSliderBarColor", &["InValue"])?,
            slider_handle_color: Function::resolve(slider_class, "SetSliderHandleColor", &["InValue"])?,
            image_texture: Function::resolve(image_class, "SetBrushFromTexture", &["Texture", "bMatchSize"])?,
            image_color: Function::resolve(image_class, "SetColorAndOpacity", &["InColorAndOpacity"])?,
            input_ui_only: Function::resolve(
                blueprint_library,
                "SetInputMode_UIOnlyEx",
                &["PlayerController", "InWidgetToFocus", "InMouseLockMode", "bFlushInput"],
            )?,
            input_game_only: Function::resolve(blueprint_library, "SetInputMode_GameOnly", &["PlayerController", "bFlushInput"])?,
            mouse_position: Function::resolve(layout_library, "GetMousePositionOnViewport", &["WorldContextObject"])?,
            viewport_size: Function::resolve(layout_library, "GetViewportSize", &["WorldContextObject"])?,
            viewport_scale: Function::resolve(layout_library, "GetViewportScale", &["WorldContextObject"])?,
            blueprint_library,
            layout_library,
            text_library,
            user_widget_class,
            canvas_class,
            border_class,
            button_class,
            text_class,
            slider_class,
            size_box_class,
            vbox_class,
            hbox_class,
            scroll_class,
            image_class,
            scale_box_class,
        })
    }

    /// A plain user widget whose whole tree is a canvas of our own. Returns the widget, its tree and the canvas.
    pub fn window(&mut self, world: Obj, controller: Obj) -> Result<(Obj, Obj, Obj), String> {
        let class = self.user_widget_class;
        let widget: *mut c_void = self
            .create
            .call(self.blueprint_library, |a| {
                a.set_obj(0, Some(world));
                a.set_obj(1, Some(class));
                a.set_obj(2, Some(controller));
            })
            .returned();
        let widget = Obj::from_raw(widget).ok_or("could not create a user widget")?;
        let tree = unsafe { widget.read_obj(self.widget_tree_offset) }.ok_or("widget has no WidgetTree")?;
        let root = ue::construct(self.canvas_class, tree)?;
        unsafe { tree.write(self.root_widget_offset, root.raw()) };
        Ok((widget, tree, root))
    }

    pub fn show_on_screen(&mut self, widget: Obj, z_order: i32) {
        self.add_to_viewport.call(widget, |a| a.set(0, z_order));
    }

    pub fn make(&self, class: Obj, tree: Obj) -> Result<Obj, String> {
        ue::construct(class, tree)
    }

    pub fn remove(&mut self, widget: Obj) {
        self.remove_from_parent.call(widget, |_| {});
    }

    pub fn visibility(&mut self, widget: Obj, visibility: u8) {
        self.set_visibility.call(widget, |a| a.set(0, visibility));
    }

    pub fn opacity(&mut self, widget: Obj, opacity: f32) {
        self.set_render_opacity.call(widget, |a| a.set(0, opacity));
    }

    pub fn clip(&mut self, widget: Obj) {
        self.set_clipping.call(widget, |a| a.set(0, CLIP_TO_BOUNDS));
    }

    pub fn angle(&mut self, widget: Obj, degrees: f32) {
        self.set_angle.call(widget, |a| a.set(0, degrees));
    }

    /// The point the widget turns around, as a fraction of its own size.
    pub fn pivot(&mut self, widget: Obj, pivot: Vec2) {
        self.set_pivot.call(widget, |a| a.set(0, pivot));
    }

    pub fn image_material(&mut self, image: Obj, material: Obj) -> Option<Obj> {
        self.image_material.call(image, |a| a.set_obj(0, Some(material)));
        let dynamic: *mut c_void = self.image_dynamic_material.call(image, |_| {}).returned();
        Obj::from_raw(dynamic)
    }

    pub fn texture_parameter(&mut self, material: Obj, name: Name, texture: Obj) {
        self.texture_parameter.call(material, |a| {
            a.set(0, name);
            a.set_obj(1, Some(texture));
        });
    }

    /// A texture made from PNG bytes.
    pub fn import_png(&mut self, world: Obj, png: &[u8]) -> Option<Obj> {
        let texture: *mut c_void = self
            .import
            .call(self.rendering, |a| {
                a.set_obj(0, Some(world));
                a.set_byte_array(1, png);
            })
            .returned();
        Obj::from_raw(texture)
    }


    fn add(&mut self, parent: Obj, child: Obj) -> Result<Obj, String> {
        let slot: *mut c_void = self.add_child.call(parent, |a| a.set_obj(0, Some(child))).returned();
        Obj::from_raw(slot).ok_or_else(|| format!("{} took no child", parent.name()))
    }

    /// Puts `child` inside a border, button, size box or scroll box.
    pub fn put(&mut self, parent: Obj, child: Obj) -> Result<(), String> {
        self.add(parent, child).map(|_| ())
    }

    pub fn hbox_add(&mut self, hbox: Obj, child: Obj, place: Place) -> Result<(), String> {
        let slot = self.add(hbox, child)?;
        self.hbox_slot.apply(slot, place);
        Ok(())
    }

    pub fn vbox_add(&mut self, vbox: Obj, child: Obj, place: Place) -> Result<(), String> {
        let slot = self.add(vbox, child)?;
        self.vbox_slot.apply(slot, place);
        Ok(())
    }

    /// Adds `child` to a canvas, anchored at `anchor` (0 to 1 on each axis) and aligned the same way.
    pub fn canvas_add(&mut self, canvas: Obj, child: Obj, anchor: Vec2, z_order: i32) -> Result<Obj, String> {
        let slot = self.add(canvas, child)?;
        self.canvas_anchors.call(slot, |a| a.set(0, Anchors { min: anchor, max: anchor }));
        self.canvas_alignment.call(slot, |a| a.set(0, anchor));
        self.canvas_z_order.call(slot, |a| a.set(0, z_order));
        Ok(slot)
    }

    /// Stretches `child` over the whole canvas.
    pub fn canvas_fill(&mut self, canvas: Obj, child: Obj, z_order: i32) -> Result<Obj, String> {
        let slot = self.add(canvas, child)?;
        let anchors = Anchors { min: Vec2 { x: 0.0, y: 0.0 }, max: Vec2 { x: 1.0, y: 1.0 } };
        self.canvas_anchors.call(slot, |a| a.set(0, anchors));
        self.canvas_position.call(slot, |a| a.set(0, Vec2::default()));
        self.canvas_size.call(slot, |a| a.set(0, Vec2::default()));
        self.canvas_z_order.call(slot, |a| a.set(0, z_order));
        Ok(slot)
    }

    pub fn place(&mut self, slot: Obj, position: Vec2, size: Option<Vec2>) {
        self.canvas_position.call(slot, |a| a.set(0, position));
        match size {
            Some(size) => {
                self.canvas_auto_size.call(slot, |a| a.set(0, false));
                self.canvas_size.call(slot, |a| a.set(0, size));
            }
            None => {
                self.canvas_auto_size.call(slot, |a| a.set(0, true));
            }
        }
    }

    /// Moves a canvas child without touching its size.
    pub fn position(&mut self, slot: Obj, position: Vec2) {
        self.canvas_position.call(slot, |a| a.set(0, position));
    }

    pub fn z_order(&mut self, slot: Obj, z_order: i32) {
        self.canvas_z_order.call(slot, |a| a.set(0, z_order));
    }

    pub fn size(&mut self, slot: Obj, size: Vec2) {
        self.canvas_size.call(slot, |a| a.set(0, size));
    }

    /// Stretches a canvas child between two fractions of its canvas, grown by `grow` on every side.
    pub fn stretch(&mut self, slot: Obj, anchors: Anchors, grow: f64) {
        self.canvas_anchors.call(slot, |a| a.set(0, anchors));
        self.canvas_offsets.call(slot, |a| a.set(0, Margin::all(-grow as f32)));
    }

    pub fn align(&mut self, slot: Obj, anchor: Vec2, alignment: Vec2) {
        self.canvas_anchors.call(slot, |a| a.set(0, Anchors { min: anchor, max: anchor }));
        self.canvas_alignment.call(slot, |a| a.set(0, alignment));
    }

    pub fn border(&mut self, tree: Obj, color: LinearColor, padding: Margin) -> Result<Obj, String> {
        let border = self.make(self.border_class, tree)?;
        self.border_color.call(border, |a| a.set(0, color));
        self.border_padding.call(border, |a| a.set(0, padding));
        Ok(border)
    }

    pub fn border_color(&mut self, border: Obj, color: LinearColor) {
        self.border_color.call(border, |a| a.set(0, color));
    }

    pub fn button(&mut self, tree: Obj, color: LinearColor) -> Result<Obj, String> {
        let button = self.make(self.button_class, tree)?;
        self.button_color.call(button, |a| a.set(0, color));
        Ok(button)
    }

    pub fn button_color(&mut self, button: Obj, color: LinearColor) {
        self.button_color.call(button, |a| a.set(0, color));
    }

    pub fn pressed(&mut self, button: Obj) -> bool {
        self.button_pressed.call(button, |_| {}).returned::<u8>() != 0
    }

    fn engine_text(&mut self, text: &str) -> Text {
        let wide: Vec<u16> = text.encode_utf16().chain([0]).collect();
        self.string_to_text.call(self.text_library, |a| a.set_string(0, &wide)).returned()
    }

    pub fn text(&mut self, tree: Obj, text: &str, size: i32, color: LinearColor, bold: bool) -> Result<Obj, String> {
        let block = self.make(self.text_class, tree)?;
        unsafe {
            block.write(self.font_offset + self.font_size_offset, size);
            if bold {
                block.write(self.font_offset + self.typeface_offset, self.bold);
            }
        }
        self.set_text(block, text);
        self.text_color(block, color);
        Ok(block)
    }

    pub fn set_text(&mut self, block: Obj, text: &str) {
        let text = self.engine_text(text);
        self.text_set.call(block, |a| a.set(0, text));
    }

    pub fn text_color(&mut self, block: Obj, color: LinearColor) {
        self.text_color.call(block, |a| a.set(0, SlateColor { color, rule: 0 }));
    }

    pub fn center_text(&mut self, block: Obj) {
        self.text_justify.call(block, |a| a.set(0, JUSTIFY_CENTER));
    }

    pub fn sized(&mut self, tree: Obj, child: Obj, width: Option<f32>, height: Option<f32>) -> Result<Obj, String> {
        let size_box = self.make(self.size_box_class, tree)?;
        if let Some(width) = width {
            self.width_override.call(size_box, |a| a.set(0, width));
        }
        if let Some(height) = height {
            self.height_override.call(size_box, |a| a.set(0, height));
        }
        self.put(size_box, child)?;
        Ok(size_box)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn slider(&mut self, tree: Obj, min: f32, max: f32, step: f32, value: f32, bar: LinearColor, handle: LinearColor) -> Result<Obj, String> {
        let slider = self.make(self.slider_class, tree)?;
        self.slider_min.call(slider, |a| a.set(0, min));
        self.slider_max.call(slider, |a| a.set(0, max));
        self.slider_step.call(slider, |a| a.set(0, step));
        self.slider_set.call(slider, |a| a.set(0, value));
        self.slider_bar_color.call(slider, |a| a.set(0, bar));
        self.slider_handle_color.call(slider, |a| a.set(0, handle));
        Ok(slider)
    }

    pub fn slider_value(&mut self, slider: Obj) -> f32 {
        self.slider_get.call(slider, |_| {}).returned()
    }

    pub fn set_slider(&mut self, slider: Obj, value: f32) {
        self.slider_set.call(slider, |a| a.set(0, value));
    }

    pub fn image(&mut self, tree: Obj, color: LinearColor) -> Result<Obj, String> {
        let image = self.make(self.image_class, tree)?;
        self.image_color.call(image, |a| a.set(0, color));
        Ok(image)
    }

    pub fn image_texture(&mut self, image: Obj, texture: Obj) {
        self.image_texture.call(image, |a| {
            a.set_obj(0, Some(texture));
            a.set(1, false);
        });
    }

    /// Mouse and keyboard go to `widget` and the cursor shows. The game resets this, so it is reapplied while open.
    pub fn take_input(&mut self, controller: Obj, widget: Obj) {
        self.input_ui_only.call(self.blueprint_library, |a| {
            a.set_obj(0, Some(controller));
            a.set_obj(1, Some(widget));
            a.set(2, DO_NOT_LOCK_MOUSE);
            a.set(3, false);
        });
        ue::set_bool_property(controller, "bShowMouseCursor", true);
    }

    pub fn release_input(&mut self, controller: Obj) {
        self.input_game_only.call(self.blueprint_library, |a| {
            a.set_obj(0, Some(controller));
            a.set(1, false);
        });
        ue::set_bool_property(controller, "bShowMouseCursor", false);
    }

    /// Mouse position in UI units from the top left of the screen.
    pub fn mouse(&mut self, world: Obj) -> Vec2 {
        self.mouse_position.call(self.layout_library, |a| a.set_obj(0, Some(world))).returned()
    }

    /// Screen size in UI units.
    pub fn screen(&mut self, world: Obj) -> Vec2 {
        let pixels: Vec2 = self.viewport_size.call(self.layout_library, |a| a.set_obj(0, Some(world))).returned();
        let scale: f32 = self.viewport_scale.call(self.layout_library, |a| a.set_obj(0, Some(world))).returned();
        let scale = if scale > 0.1 { f64::from(scale) } else { 1.0 };
        Vec2 { x: pixels.x / scale, y: pixels.y / scale }
    }
}
