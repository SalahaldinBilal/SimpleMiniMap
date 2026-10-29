use crate::host;
use crate::ue::{self, Function, Obj, Rotator, Vec3, Weak};

const NON_GAME_WORLDS: [&str; 3] = ["PL_PPSplash", "PL_Login", "PL_Title"];

pub struct Player {
    pub world: Obj,
    pub controller: Obj,
    pub pawn: Obj,
    pub location: Vec3,
    pub yaw: f64,
    pub camera_yaw: f64,
    pub screen_open: bool,
    /// The game is fading the screen, as it does while teleporting, sleeping or playing a cutscene.
    pub fading: bool,
    pub in_base: bool,
    /// Inside a dungeon, boss room or arena, which the game places away from the overworld, often under it.
    pub in_stage: bool,
}

pub struct Game {
    statics: Obj,
    utility: Obj,
    get_player_controller: Function,
    get_location: Function,
    get_rotation: Function,
    get_hud_service: Function,
    overlay_active: Function,
    fade_active: Function,
    control_rotation: Function,
    inside_base: Function,
    inside_stage: Function,
    player_character_class: Obj,
    base_check_offset: usize,
    viewport_offset: usize,
    world_offset: usize,
    pawn_offset: usize,
    current_world: Option<(Weak, bool)>,
    hud_service: Option<Weak>,
}

impl Game {
    pub fn new(engine: Obj) -> Result<Self, String> {
        let engine_class = engine.class().ok_or("engine has no class")?;
        let viewport_class = ue::find("/Script/Engine.GameViewportClient")?;
        let controller_class = ue::find("/Script/Engine.Controller")?;
        let actor_class = ue::find("/Script/Engine.Actor")?;
        let statics = ue::find("/Script/Engine.Default__GameplayStatics")?;
        let utility = ue::find("/Script/Pal.Default__PalUtility")?;
        let hud_service_class = ue::find("/Script/Pal.PalHUDService")?;
        let player_character_class = ue::find("/Script/Pal.PalPlayerCharacter")?;
        let base_check_class = ue::find("/Script/Pal.PalInsideBaseCampCheckComponent")?;
        Ok(Self {
            control_rotation: Function::resolve(controller_class, "GetControlRotation", &[])?,
            inside_base: Function::resolve(base_check_class, "IsInsideBaseCamp", &[])?,
            inside_stage: Function::resolve(utility, "IsInsideStage", &["TargetActor"])?,
            base_check_offset: ue::property_offset(player_character_class, "InsideBaseCampCheckComponent")?,
            player_character_class,
            get_player_controller: Function::resolve(statics, "GetPlayerController", &["WorldContextObject", "PlayerIndex"])?,
            get_location: Function::resolve(actor_class, "K2_GetActorLocation", &[])?,
            get_rotation: Function::resolve(actor_class, "K2_GetActorRotation", &[])?,
            get_hud_service: Function::resolve(utility, "GetHUDService", &["WorldContextObject"])?,
            overlay_active: Function::resolve(hud_service_class, "IsAnyOverlayUIActive", &[])?,
            fade_active: Function::resolve(hud_service_class, "IsAnyFadeWidgetActive", &[])?,
            viewport_offset: ue::property_offset(engine_class, "GameViewport")?,
            world_offset: ue::property_offset(viewport_class, "World")?,
            pawn_offset: ue::property_offset(controller_class, "Pawn")?,
            statics,
            utility,
            current_world: None,
            hud_service: None,
        })
    }

    pub fn player(&mut self, engine: Obj) -> Option<Player> {
        let viewport = unsafe { engine.read_obj(self.viewport_offset) }?;
        let world = unsafe { viewport.read_obj(self.world_offset) }?;
        if !self.is_play_world(world) {
            return None;
        }
        let controller: *mut std::ffi::c_void = self
            .get_player_controller
            .call(self.statics, |args| {
                args.set_obj(0, Some(world));
                args.set(1, 0i32);
            })
            .returned();
        let controller = Obj::from_raw(controller)?;
        let pawn = unsafe { controller.read_obj(self.pawn_offset) }?;
        let location: Vec3 = self.get_location.call(pawn, |_| {}).returned();
        let rotation: Rotator = self.get_rotation.call(pawn, |_| {}).returned();
        let camera: Rotator = self.control_rotation.call(controller, |_| {}).returned();
        let (screen_open, fading) = self.screen_state(world);
        let in_base = self.in_base(pawn);
        let utility = self.utility;
        let in_stage = self.inside_stage.call(utility, |args| args.set_obj(0, Some(pawn))).returned::<u8>() != 0;
        Some(Player {
            world,
            controller,
            pawn,
            location,
            yaw: rotation.yaw,
            camera_yaw: camera.yaw,
            screen_open,
            fading,
            in_base,
            in_stage,
        })
    }

    /// The game's own check against the base's real boundary.
    fn in_base(&mut self, pawn: Obj) -> bool {
        if !ue::is_a(pawn, self.player_character_class) {
            return false;
        }
        let Some(check) = (unsafe { pawn.read_obj(self.base_check_offset) }) else { return false };
        self.inside_base.call(check, |_| {}).returned::<u8>() != 0
    }

    /// Whether a game screen is open, and whether the game is fading the screen.
    fn screen_state(&mut self, world: Obj) -> (bool, bool) {
        let service = match self.hud_service.and_then(Weak::get) {
            Some(service) => service,
            None => {
                let service: *mut std::ffi::c_void = self
                    .get_hud_service
                    .call(self.utility, |args| args.set_obj(0, Some(world)))
                    .returned();
                let Some(service) = Obj::from_raw(service) else { return (false, false) };
                self.hud_service = Some(Weak::new(service));
                service
            }
        };
        let screen_open = self.overlay_active.call(service, |_| {}).returned::<u8>() != 0;
        let fading = self.fade_active.call(service, |_| {}).returned::<u8>() != 0;
        (screen_open, fading)
    }

    fn is_play_world(&mut self, world: Obj) -> bool {
        if let Some((weak, playable)) = self.current_world {
            if weak.get() == Some(world) {
                return playable;
            }
        }
        let name = world.name();
        let playable = !NON_GAME_WORLDS.contains(&name.as_str());
        host::info(&format!("world {name} ({})", if playable { "playable" } else { "menu" }));
        self.current_world = Some((Weak::new(world), playable));
        playable
    }
}
