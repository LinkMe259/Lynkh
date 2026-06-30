#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum CoverArt {
    TacticalBlue,
    MiningPink,
    CompetitiveCyan,
    StealthGreen,
    AmberOps,
    Default,
}

#[derive(Clone, Copy)]
pub enum ProgramIcon {
    Gamepad,
    Shield,
    Radar,
    Bolt,
}

#[derive(Clone, Copy)]
pub struct LaunchStep {
    pub label: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
}

#[derive(Clone, Copy)]
pub struct ProgramProfile {
    pub id: &'static str,
    pub match_names: &'static [&'static str],
    pub subtitle: &'static str,
    pub cover_image: Option<&'static str>,
    pub cover_art: CoverArt,
    pub icon: ProgramIcon,
    pub launch_steps: &'static [LaunchStep],
}

pub const DEFAULT_PROFILE: ProgramProfile = ProgramProfile {
    id: "default",
    match_names: &[],
    subtitle: "",
    cover_image: None,
    cover_art: CoverArt::Default,
    icon: ProgramIcon::Gamepad,
    launch_steps: &[],
};

pub const PROGRAM_PROFILES: &[ProgramProfile] = &[
    // เพิ่มโปรแกรมใหม่: copy block นี้ แล้วแก้ match_names / cover_image / launch_steps
    ProgramProfile {
        id: "design_suite_pro",
        match_names: &["Design Suite Pro", "Design Suite"],
        subtitle: "Creative tools access",
        cover_image: Some("assets/program_covers/design_suite_pro.png"),
        cover_art: CoverArt::TacticalBlue,
        icon: ProgramIcon::Shield,
        launch_steps: &[LaunchStep {
            label: "Run design suite",
            command: r"C:\NovaLoader\design_suite_pro.exe",
            args: &[],
        }],
    },
    ProgramProfile {
        id: "fps_optimizer_x",
        match_names: &["FPS Optimizer X", "FPS Optimizer", "Optimizer X"],
        subtitle: "Performance profile",
        cover_image: Some("assets/program_covers/fps_optimizer_x.png"),
        cover_art: CoverArt::CompetitiveCyan,
        icon: ProgramIcon::Bolt,
        launch_steps: &[LaunchStep {
            label: "Run FPS optimizer",
            command: r"C:\NovaLoader\fps_optimizer_x.exe",
            args: &[],
        }],
    },
    ProgramProfile {
        id: "game_booster_pro",
        match_names: &["Game Booster Pro", "Game Booster"],
        subtitle: "Booster access package",
        cover_image: Some("assets/program_covers/game_booster_pro.png"),
        cover_art: CoverArt::StealthGreen,
        icon: ProgramIcon::Gamepad,
        launch_steps: &[LaunchStep {
            label: "Run game booster",
            command: r"C:\NovaLoader\game_booster_pro.exe",
            args: &[],
        }],
    },
    ProgramProfile {
        id: "warface",
        match_names: &["WARFACE", "Warface", "Ahohc Warface"],
        subtitle: "Tactical loader profile",
        cover_image: Some("assets/program_covers/warface.png"),
        cover_art: CoverArt::TacticalBlue,
        icon: ProgramIcon::Shield,
        launch_steps: &[
            LaunchStep {
                label: "Run game loader",
                command: r"C:\NovaLoader\warface_loader.exe",
                args: &[],
            },
            LaunchStep {
                label: "Start companion",
                command: r"C:\NovaLoader\warface_companion.exe",
                args: &["--silent"],
            },
        ],
    },
    ProgramProfile {
        id: "mining",
        match_names: &["MINING", "Mining Event", "Hafif Madencilik"],
        subtitle: "Mining event utilities",
        cover_image: Some("assets/program_covers/mining.png"),
        cover_art: CoverArt::MiningPink,
        icon: ProgramIcon::Bolt,
        launch_steps: &[LaunchStep {
            label: "Run mining tool",
            command: r"C:\NovaLoader\mining_tool.exe",
            args: &[],
        }],
    },
    ProgramProfile {
        id: "competitive",
        match_names: &["COMPETITIVE", "Rekabet", "Competitive Mode"],
        subtitle: "Competitive mode package",
        cover_image: Some("assets/program_covers/competitive.png"),
        cover_art: CoverArt::CompetitiveCyan,
        icon: ProgramIcon::Radar,
        launch_steps: &[LaunchStep {
            label: "Run competitive loader",
            command: r"C:\NovaLoader\competitive_loader.exe",
            args: &["--mode", "ranked"],
        }],
    },
];

pub fn profile_for(program_name: &str) -> &'static ProgramProfile {
    PROGRAM_PROFILES
        .iter()
        .find(|profile| profile.matches(program_name))
        .unwrap_or(&DEFAULT_PROFILE)
}

impl ProgramProfile {
    fn matches(&self, program_name: &str) -> bool {
        let normalized = normalize(program_name);
        self.match_names.iter().any(|name| {
            let candidate = normalize(name);
            normalized == candidate || normalized.contains(&candidate)
        })
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
