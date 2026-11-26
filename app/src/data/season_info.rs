pub struct Operator {
    pub name: &'static str,
    pub gadget_name: &'static str,
    pub gadget_desc: &'static str,
    pub img: Option<&'static str>,
}

pub struct Map {
    pub name: &'static str,
    pub img: Option<&'static str>,
}

pub struct Entry {
    pub key: &'static str,
    pub release: &'static str,
    pub operators: &'static [Operator],
    pub maps: &'static [Map],
    pub highlights: &'static [&'static str],
}

impl Operator {
    pub fn picture(&self) -> String {
        picture("ops", self.name, self.img)
    }
}

impl Map {
    pub fn picture(&self) -> String {
        picture("maps", self.name, self.img)
    }
}

fn picture(folder: &str, name: &str, img: Option<&str>) -> String {
    let file = img.map_or_else(|| slug(name), str::to_owned);
    format!("info/{folder}/{file}.webp")
}

fn slug(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|letter| match letter {
            'ã' | 'ä' => 'a',
            'ó' | 'ø' => 'o',
            ' ' => '-',
            plain => plain,
        })
        .collect()
}

pub fn entry(key: &str) -> Option<&'static Entry> {
    SEASON_INFO.iter().find(|entry| entry.key == key)
}

pub static SEASON_INFO: &[Entry] = &[
    Entry {
        key: "Y1S0_Vanilla",
        release: "Dec 1, 2015",
        operators: &[],
        maps: &[],
        highlights: &[],
    },
    Entry {
        key: "Y1S1_BlackIce",
        release: "Feb 2, 2016",
        operators: &[
            Operator {
                name: "Buck",
                gadget_name: "Skeleton Key",
                gadget_desc: "adds an under-barrel 12 gauge shotgun for breaching",
                img: None,
            },
            Operator {
                name: "Frost",
                gadget_name: "Welcome Mat",
                gadget_desc: "floor trap that downs attackers who step on it",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Yacht",
            img: None,
        }],
        highlights: &[
            "Option for attackers to pick their own spawn location",
            "Stun Grenade effect range roughly doubled",
            "Spectator Camera with keyboard and mouse controls",
        ],
    },
    Entry {
        key: "Y1S2_DustLine",
        release: "May 10, 2016",
        operators: &[
            Operator {
                name: "Blackbeard",
                gadget_name: "TARS Rifle Shield",
                gadget_desc: "transparent ballistic shield mounted on his rifle",
                img: Some("blackbeard-og"),
            },
            Operator {
                name: "Valkyrie",
                gadget_name: "Black Eye",
                gadget_desc: "throwable sticky camera that gives defenders a live feed",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Border",
            img: Some("border-y1s2"),
        }],
        highlights: &[
            "Loadout changes between rounds",
            "Montagne gains side protection on his shield",
            "Stun Grenade detonates 1s after impact",
        ],
    },
    Entry {
        key: "Y1S3_SkullRain",
        release: "Aug 2, 2016",
        operators: &[
            Operator {
                name: "Capitão",
                gadget_name: "Tactical Crossbow",
                gadget_desc: "fires silent asphyxiating bolts and micro smoke grenades",
                img: None,
            },
            Operator {
                name: "Caveira",
                gadget_name: "Silent Step",
                gadget_desc: "makes her movement nearly silent while active",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Favela",
            img: Some("favela-y1s3"),
        }],
        highlights: &[
            "**Claymore** for attackers, **Impact Grenade** for defenders",
            "Twitch, Doc and Blackbeard reworked",
            "Angled Grip attachment for faster ADS transitions",
            "**Tactical Realism** custom game mode with minimal HUD",
        ],
    },
    Entry {
        key: "Y1S4_RedCrow",
        release: "Nov 17, 2016",
        operators: &[
            Operator {
                name: "Hibana",
                gadget_name: "X-KAIROS",
                gadget_desc: "launches explosive pellets that breach reinforced walls",
                img: Some("hibana-og"),
            },
            Operator {
                name: "Echo",
                gadget_name: "Yokai",
                gadget_desc: "ceiling-clinging drone that fires disorienting ultrasonic bursts",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Skyscraper",
            img: Some("skyscraper-y1s4"),
        }],
        highlights: &[
            "Caliber-based destruction — stronger rounds make bigger holes",
            "Neck shots count as headshots, limbs take full damage",
            "ACOG removed from the SMG-11",
        ],
    },
    Entry {
        key: "Y2S1_VelvetShell",
        release: "Feb 7, 2017",
        operators: &[
            Operator {
                name: "Jackal",
                gadget_name: "Eyenox Model III",
                gadget_desc: "scans enemy footprints to track and ping their location",
                img: None,
            },
            Operator {
                name: "Mira",
                gadget_name: "Black Mirror",
                gadget_desc: "bulletproof, ejectable one-way mirror for reinforced or soft walls",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Coastline",
            img: Some("coastline-y2s1"),
        }],
        highlights: &[
            "Sight positions harmonized across all weapons",
            "Capitão gets a Claymore, IQ gets Frag Grenades",
        ],
    },
    Entry {
        key: "Y2S2_Health",
        release: "Jun 7, 2017",
        operators: &[],
        maps: &[],
        highlights: &[
            "Maintenance season with no new operators or maps",
            "Hitboxes cover only the body, not gear or headgear",
            "Ash and Thermite get Stun Grenades, Fuze and Jackal Smoke Grenades",
            "Glaz gets a weaker OTs-03 after his thermal scope rework",
        ],
    },
    Entry {
        key: "Y2S3_BloodOrchid",
        release: "Sep 5, 2017",
        operators: &[
            Operator {
                name: "Ying",
                gadget_name: "Candela",
                gadget_desc: "releases a cluster of flash charges to blind enemies",
                img: None,
            },
            Operator {
                name: "Lesion",
                gadget_name: "Gu Mine",
                gadget_desc: "injects a toxin that damages and slows enemies",
                img: None,
            },
            Operator {
                name: "Ela",
                gadget_name: "Grzmot Mine",
                gadget_desc: "proximity concussion mine that dazes and impairs hearing",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Theme Park",
            img: Some("theme-park-y2s3"),
        }],
        highlights: &[
            "Attackers can deploy both drones at the same time",
            "ACOG removed from 2- and 3-speed defenders",
            "Smoke Grenades made opaque, cut to 2 per operator",
            "Lighting and sky rework on all maps, with less bloom",
        ],
    },
    Entry {
        key: "Y2S4_WhiteNoise",
        release: "Dec 5, 2017",
        operators: &[
            Operator {
                name: "Dokkaebi",
                gadget_name: "Logic Bomb",
                gadget_desc: "hacks defender phones to ring and reveal their positions",
                img: Some("dokkaebi-og"),
            },
            Operator {
                name: "Vigil",
                gadget_name: "ERC-7",
                gadget_desc: "wipes his image from any cameras in view",
                img: None,
            },
            Operator {
                name: "Zofia",
                gadget_name: "KS79 Lifeline",
                gadget_desc: "fires concussion and impact grenades from a launcher",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Tower",
            img: None,
        }],
        highlights: &[
            "New pistol recoil animation and a higher fire rate",
            "Thrown grenades inherit player movement",
            "Color-coded HUD icons for buffs and debuffs",
        ],
    },
    Entry {
        key: "Y3S1_Chimera",
        release: "Mar 6, 2018",
        operators: &[
            Operator {
                name: "Lion",
                gadget_name: "EE-ONE-D",
                gadget_desc: "aerial drone scans and pings moving enemies through walls",
                img: None,
            },
            Operator {
                name: "Finka",
                gadget_name: "Adrenal Surge",
                gadget_desc: "boosts team health and revives downed allies",
                img: None,
            },
        ],
        maps: &[],
        highlights: &[
            "Reload rework — ADS interrupt and resume points",
            "**Raw Input** option for mouse and keyboard",
            "Faster Blitz with less armor, weaker Scorpion for Ela",
        ],
    },
    Entry {
        key: "Y3S2_ParaBellum",
        release: "Jun 7, 2018",
        operators: &[
            Operator {
                name: "Alibi",
                gadget_name: "Prisma",
                gadget_desc: "deploys holograms that ping enemies who shoot or touch them",
                img: None,
            },
            Operator {
                name: "Maestro",
                gadget_name: "Evil Eye",
                gadget_desc: "deploys bulletproof remote cameras that fire laser bursts",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Villa",
            img: Some("villa-y3s2"),
        }],
        highlights: &[
            "**Bulletproof Camera** secondary gadget for defenders",
            "**Pick & Ban** in custom games, up to 4 operator bans",
            "Going prone from standing breaks ADS, countering dropshots",
            "Second Yokai drone for Echo, Clubhouse map buff",
        ],
    },
    Entry {
        key: "Y3S3_GrimSky",
        release: "Sep 4, 2018",
        operators: &[
            Operator {
                name: "Maverick",
                gadget_name: "Breaching Torch",
                gadget_desc: "silently burns small holes through reinforced walls and hatches",
                img: None,
            },
            Operator {
                name: "Clash",
                gadget_name: "CCE Shield",
                gadget_desc: "extendable shield that tasers enemies to slow them down",
                img: Some("clash-og"),
            },
        ],
        maps: &[Map {
            name: "Hereford Base",
            img: None,
        }],
        highlights: &[
            "Sight misalignment fixes and hatch destruction rework",
            "Consulate buff with a fourth bomb site and safer spawns",
            "Thatcher disables cameras only temporarily",
        ],
    },
    Entry {
        key: "Y3S4_WindBastion",
        release: "Dec 4, 2018",
        operators: &[
            Operator {
                name: "Nomad",
                gadget_name: "Airjab Launcher",
                gadget_desc: "launches repulsion grenades that knock back nearby enemies",
                img: None,
            },
            Operator {
                name: "Kaid",
                gadget_name: "Rtila Electroclaw",
                gadget_desc: "electrifies reinforced walls, hatches and barbed wire",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Fortress",
            img: Some("fortress-y3s4"),
        }],
        highlights: &[
            "New throw curves for all throwables except the Nitro Cell",
            "SMG-11 machine pistol for Mute",
        ],
    },
    Entry {
        key: "Y4S1_BurntHorizon",
        release: "Mar 6, 2019",
        operators: &[
            Operator {
                name: "Gridlock",
                gadget_name: "Trax Stingers",
                gadget_desc: "deploys spreading spike traps that slow and damage enemies",
                img: None,
            },
            Operator {
                name: "Mozzie",
                gadget_name: "Pest Launcher",
                gadget_desc: "launches Pests that hack and steal attacker drones",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Outback",
            img: Some("outback-y4s1"),
        }],
        highlights: &[
            "Revived operators get 20 HP instead of 50",
            "Lean camera moved to the center of the head",
            "Faster, safer Breach Charges",
            "Mute blocks calls from Dokkaebi",
        ],
    },
    Entry {
        key: "Y4S2_PhantomSight",
        release: "Jun 11, 2019",
        operators: &[
            Operator {
                name: "Nøkk",
                gadget_name: "HEL Presence Reduction",
                gadget_desc: "hides her from observation tools and muffles her footsteps",
                img: None,
            },
            Operator {
                name: "Warden",
                gadget_name: "Glance Smart Glasses",
                gadget_desc: "grants vision through smoke and protection from flashes",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Kafe Dostoyevsky",
            img: Some("kafe-dostoyevsky-y4s2"),
        }],
        highlights: &[
            "Glaz must stand still to see through smoke",
            "Cameras and drones light up in team colors",
            "HUD stays visible when flashed or stunned",
            "Rounds can no longer end in a draw",
        ],
    },
    Entry {
        key: "Y4S3_EmberRise",
        release: "Sep 11, 2019",
        operators: &[
            Operator {
                name: "Amaru",
                gadget_name: "Garra Hook",
                gadget_desc: "grapples to ledges, windows and open hatches for fast entry",
                img: None,
            },
            Operator {
                name: "Goyo",
                gadget_name: "Volcán Shield",
                gadget_desc: "deployable shield with an incendiary charge on the back",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Kanal",
            img: None,
        }],
        highlights: &[
            "Secondary gadgets swapped for 11 operators",
            "Redesigned Deployable Shield fits doorframes",
            "Shield ADS time raised from 0.4s to 0.6s",
            "Terrorist Hunt without bombers or trap rooms",
        ],
    },
    Entry {
        key: "Y4S4_ShiftingTides",
        release: "Dec 3, 2019",
        operators: &[
            Operator {
                name: "Kali",
                gadget_name: "LV Explosive Lance",
                gadget_desc: "fires a lance that destroys gadgets on both sides of walls",
                img: None,
            },
            Operator {
                name: "Wamai",
                gadget_name: "Mag-NET System",
                gadget_desc: "attracts enemy projectiles and detonates them near itself",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Theme Park",
            img: Some("theme-park-y4s4"),
        }],
        highlights: &[
            "Limb penetration for most weapons",
            "Rappel exits need manual confirmation",
            "Jackal gets pings based on footprint age",
        ],
    },
    Entry {
        key: "Y5S1_VoidEdge",
        release: "Mar 10, 2020",
        operators: &[
            Operator {
                name: "Iana",
                gadget_name: "Gemini Replicator",
                gadget_desc: "projects a remote-controlled holographic clone of Iana",
                img: None,
            },
            Operator {
                name: "Oryx",
                gadget_name: "Remah Dash",
                gadget_desc: "dashes to knock down enemies and smash through soft walls",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Oregon",
            img: Some("oregon-y5s1"),
        }],
        highlights: &[
            "Explosions deal shrapnel damage that cover blocks",
            "Drones spawn on the chosen spawn side",
            "Twitch, Lesion and Warden reworked",
            "Barricade debris cleanup for consistent sightlines",
        ],
    },
    Entry {
        key: "Y5S2_SteelWave",
        release: "Jun 16, 2020",
        operators: &[
            Operator {
                name: "Ace",
                gadget_name: "S.E.L.M.A. Aqua Breacher",
                gadget_desc: "uses hydraulic pressure to breach reinforced surfaces",
                img: None,
            },
            Operator {
                name: "Melusi",
                gadget_name: "Banshee Sonic Defense",
                gadget_desc: "emits noise and slows attackers in range and line of sight",
                img: None,
            },
        ],
        maps: &[Map {
            name: "House",
            img: None,
        }],
        highlights: &[
            "**Proximity Alarm** secondary gadget for defenders",
            "Amaru breaks through hatches with the Garra Hook",
            "Concussion no longer rolls the camera or alters sensitivity",
            "ACS12 shotgun switched to slugs",
        ],
    },
    Entry {
        key: "Y5S3_ShadowLegacy",
        release: "Sep 10, 2020",
        operators: &[
            Operator {
                name: "Zero",
                gadget_name: "ARGUS Launcher",
                gadget_desc: "launches cameras that pierce walls and fire laser shots",
                img: None,
            },
            Operator {
                name: "Tachanka",
                gadget_name: "Shumikha Launcher",
                gadget_desc: "launches incendiary grenades that ignite after bouncing",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Chalet",
            img: Some("chalet-y5s3"),
        }],
        highlights: &[
            "Defenders share a **Reinforcement Pool** of 10",
            "**Hard Breach Charge** secondary gadget for attackers",
            "**Ping 2.0** contextual pings, also on cams and after death",
            "DP27 light machine gun for Tachanka",
        ],
    },
    Entry {
        key: "Y5S4_NeonDawn",
        release: "Dec 1, 2020",
        operators: &[
            Operator {
                name: "Aruni",
                gadget_name: "Surya Gate",
                gadget_desc: "deploys a laser gate that damages attackers passing through",
                img: None,
            },
            Operator {
                name: "Jäger",
                gadget_name: "Active Defense System",
                gadget_desc: "unlimited projectile intercepts on a 10 second cooldown",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Skyscraper",
            img: Some("skyscraper-y5s4"),
        }],
        highlights: &[
            "Runout detection timer cut from 2s to 1s",
            "Echo can no longer cloak his Yokai drones",
            "Hibana can fire 2, 4 or 6 X-KAIROS pellets at once",
            "Reinforcements deploy in 4.5s instead of 5.5s",
        ],
    },
    Entry {
        key: "Y6S1_CrimsonHeist",
        release: "Mar 16, 2021",
        operators: &[Operator {
            name: "Flores",
            gadget_name: "RCE-Ratero Charge",
            gadget_desc: "remote-controlled explosive drone that destroys gadgets",
            img: None,
        }],
        maps: &[Map {
            name: "Border",
            img: Some("border-y6s1"),
        }],
        highlights: &[
            "GONNE-6 explosive hand cannon for 8 attackers",
            "**Match Replay** to re-watch matches from any angle",
            "Mute also disables Claymores and Airjabs",
            "Tactical Realism removed from custom games",
        ],
    },
    Entry {
        key: "Y6S2_NorthStar",
        release: "Jun 14, 2021",
        operators: &[Operator {
            name: "Thunderbird",
            gadget_name: "Kóna Station",
            gadget_desc: "deployable station that heals or revives nearby operators",
            img: None,
        }],
        maps: &[Map {
            name: "Favela",
            img: None,
        }],
        highlights: &[
            "Single bullet holes in walls no longer give line of sight",
            "Melee shatters Black Mirrors, Evil Eyes and Bulletproof Cameras",
            "Gas from Smoke no longer spreads through walls and floors",
            "Bodies of eliminated operators replaced by icons",
        ],
    },
    Entry {
        key: "Y6S3_CrystalGuard",
        release: "Sep 7, 2021",
        operators: &[Operator {
            name: "Osa",
            gadget_name: "Talon-8 Clear Shield",
            gadget_desc: "transparent bulletproof shield she carries or deploys",
            img: None,
        }],
        maps: &[
            Map {
                name: "Bank",
                img: Some("bank-y6s3"),
            },
            Map {
                name: "Coastline",
                img: Some("coastline-y2s1"),
            },
            Map {
                name: "Clubhouse",
                img: Some("clubhouse-y6s3"),
            },
        ],
        highlights: &[
            "Armor converted to health — 100, 110 or 125 HP",
            "Twitch gets a jumping Shock Drone that destroys gadgets",
            "Linear damage drop-off, flat 15% suppressor penalty",
            "Attackers always pick their own spawn location",
        ],
    },
    Entry {
        key: "Y6S4_HighCalibre",
        release: "Nov 30, 2021",
        operators: &[Operator {
            name: "Thorn",
            gadget_name: "Razorbloom Shell",
            gadget_desc: "sticks to surfaces and bursts lethal blades near enemies",
            img: None,
        }],
        maps: &[Map {
            name: "Outback",
            img: None,
        }],
        highlights: &[
            "Bulletproof Camera rotates and fires EMP bursts",
            "HUD rework, drone counter and customizable team colors",
            "Finka can revive herself with Adrenal Surge",
        ],
    },
    Entry {
        key: "Y7S1_DemonVeil",
        release: "Mar 15, 2022",
        operators: &[
            Operator {
                name: "Azami",
                gadget_name: "Kiba Barrier",
                gadget_desc: "thrown kunai expands into a bulletproof barrier",
                img: None,
            },
            Operator {
                name: "Goyo",
                gadget_name: "Volcán Canister",
                gadget_desc: "incendiary canister that ignites when shot",
                img: None,
            },
        ],
        maps: &[Map {
            name: "Emerald Plains",
            img: None,
        }],
        highlights: &[
            "**Attacker Repick** during the prep phase",
            "**Team Deathmatch** mode, playable in custom games",
            "All non-magnifying sights available on most weapons",
            "Outdoor defender cameras lose signal after 10s",
        ],
    },
    Entry {
        key: "Y7S2_VectorGlare",
        release: "Jun 14, 2022",
        operators: &[Operator {
            name: "Sens",
            gadget_name: "R.O.U. Projector System",
            gadget_desc: "rolls and projects a light wall that blocks lines of sight",
            img: None,
        }],
        maps: &[Map {
            name: "Close Quarter",
            img: None,
        }],
        highlights: &[
            "**Shooting Range** with recoil and damage lanes",
            "Third secondary weapon option for 9 operators",
        ],
    },
    Entry {
        key: "Y7S3_BrutalSwarm",
        release: "Sep 6, 2022",
        operators: &[Operator {
            name: "Grim",
            gadget_name: "Kawan Hive Launcher",
            gadget_desc: "launches bot swarms that reveal enemies passing through",
            img: None,
        }],
        maps: &[Map {
            name: "Stadium Bravo",
            img: Some("stadium-bravo-y7s3"),
        }],
        highlights: &[
            "Recoil rework — progressive recoil",
            "**Impact EMP Grenade** secondary gadget for attackers",
            "Suppressors no longer reduce damage, more attachment options",
            "Rook armor plates grant Withstand when downed",
        ],
    },
    Entry {
        key: "Y7S4_SolarRaid",
        release: "Dec 6, 2022",
        operators: &[Operator {
            name: "Solis",
            gadget_name: "SPEC-IO Electro-Sensor",
            gadget_desc: "detects and marks enemy electronic devices",
            img: None,
        }],
        maps: &[Map {
            name: "Nighthaven Labs",
            img: Some("nighthaven-labs-y7s4"),
        }],
        highlights: &[
            "All operators move at the same speed while aiming",
            "New health and speed ratings for 13 operators",
            "Extended Barrel adds 15% weapon damage",
            "No prep-phase friendly fire, optional in custom games",
        ],
    },
    Entry {
        key: "Y8S1_CommandingForce",
        release: "Mar 7, 2023",
        operators: &[Operator {
            name: "Brava",
            gadget_name: "Kludge Drone",
            gadget_desc: "takes over enemy devices or destroys them if uncontrollable",
            img: None,
        }],
        maps: &[],
        highlights: &[
            "Reload rework — interrupted reloads leave no magazine",
            "Zero triggers the ARGUS pierce manually",
            "Compensator and Muzzle Brake recoil reduction buffed",
        ],
    },
    Entry {
        key: "Y8S2_DreadFactor",
        release: "May 30, 2023",
        operators: &[Operator {
            name: "Fenrir",
            gadget_name: "F-NATT Dread Mine",
            gadget_desc: "releases fear gas that severely limits enemy vision",
            img: None,
        }],
        maps: &[Map {
            name: "Consulate",
            img: Some("consulate-y8s2"),
        }],
        highlights: &[
            "**Observation Blocker** secondary gadget for defenders",
            "Shooting Range aiming lane with moving targets",
            "Host button to randomize teams in custom games",
        ],
    },
    Entry {
        key: "Y8S3_HeavyMettle",
        release: "Aug 29, 2023",
        operators: &[
            Operator {
                name: "Ram",
                gadget_name: "BU-GI Auto-Breacher",
                gadget_desc: "mini-tank that destroys breakable surfaces in its path",
                img: None,
            },
            Operator {
                name: "Frost",
                gadget_name: "Welcome Mat",
                gadget_desc: "floor trap that downs attackers, placeable under barbed wire",
                img: None,
            },
        ],
        maps: &[],
        highlights: &[
            "Shotgun rework and bouncing Kawan Hives for Grim",
            "Lesion rework — visible Gu Mines with impact damage",
            "**Weapon Roulette** arcade mode",
            "Free camera and hideable HUD for spectators",
        ],
    },
    Entry {
        key: "Y8S4_DeepFreeze",
        release: "Dec 6, 2023",
        operators: &[Operator {
            name: "Tubarão",
            gadget_name: "Zoto Canister",
            gadget_desc: "throwable canister that freezes devices and slows enemies",
            img: None,
        }],
        maps: &[Map {
            name: "Lair",
            img: Some("lair-y8s4"),
        }],
        highlights: &[
            "Frag Grenades lose cooking, added to 5 more attackers",
            "**Versus AI** and **Map Training** replace Situations and Training Grounds",
            "Up to 4 spectators per match, switchable from the lobby",
            "Controller remapping and deadzone customization",
        ],
    },
    Entry {
        key: "Y9S1_DeadlyOmen",
        release: "Mar 12, 2024",
        operators: &[Operator {
            name: "Deimos",
            gadget_name: "DeathMARK Tracker",
            gadget_desc: "probe seeks a marked enemy and reveals their location",
            img: None,
        }],
        maps: &[],
        highlights: &[
            "Shield rework — sprint, free look, suppressive fire, no hip fire",
            "Slower ADS for all weapons",
            "Attachment rework — Horizontal Grip and new scope zooms",
        ],
    },
    Entry {
        key: "Y9S2_NewBlood",
        release: "Jun 11, 2024",
        operators: &[
            Operator {
                name: "Striker",
                gadget_name: "Gadget Kit",
                gadget_desc: "lets Striker carry two attacker secondary gadgets",
                img: None,
            },
            Operator {
                name: "Sentry",
                gadget_name: "Gadget Kit",
                gadget_desc: "equips two different defender secondary gadgets",
                img: None,
            },
        ],
        maps: &[],
        highlights: &[
            "Fenrir and Solis nerfed — fewer mines, SPEC-IO off in prep",
            "Barbed Wire deals 5 HP/s to attackers moving in it",
            "Stadium Alpha and Bravo updated, bulletproof glass removed",
            "**Endless Drill** warm-up mode with respawning enemies",
        ],
    },
    Entry {
        key: "Y9S3_TwinShells",
        release: "Sep 10, 2024",
        operators: &[Operator {
            name: "Skopós",
            gadget_name: "V10 Pantheon Shells",
            gadget_desc: "swaps control between two robotic shells at will",
            img: None,
        }],
        maps: &[],
        highlights: &[
            "**Drone Boost** for faster drone movement",
            "DX12 as the default graphics API, Vulkan removed",
            "1v1 custom game presets — **Short Match** and **Long Match**",
            "**Versus AI 2.0** — defend against AI attackers",
        ],
    },
    Entry {
        key: "Y9S4_CollisionPoint",
        release: "Dec 3, 2024",
        operators: &[Operator {
            name: "Blackbeard",
            gadget_name: "H.U.L.L. Adaptable Shield",
            gadget_desc: "deployable shield he raises to block incoming fire",
            img: None,
        }],
        maps: &[],
        highlights: &[
            "Shields nerfed — melee damage removed, earlier suppressive fire",
            "Stun Grenades reduced to 2 per operator",
            "Sens can toggle R.O.U. walls, which block thermal sights",
        ],
    },
    Entry {
        key: "Y10S1_PrepPhase",
        release: "Mar 4, 2025",
        operators: &[Operator {
            name: "Rauora",
            gadget_name: "D.O.M. Panel Launcher",
            gadget_desc: "launches bulletproof panels onto doorways from a distance",
            img: None,
        }],
        maps: &[],
        highlights: &[
            "DX12 as the only graphics API, DX11 removed",
            "Map Training adds Presidential Plane, Yacht and Favela",
        ],
    },
    Entry {
        key: "Y10S2_Daybreak",
        release: "Jun 10, 2025",
        operators: &[Operator {
            name: "Clash",
            gadget_name: "CCE Shield MK2",
            gadget_desc: "electrified shield that slows attackers, anchorable in place",
            img: None,
        }],
        maps: &[Map {
            name: "District",
            img: None,
        }],
        highlights: &[
            "**Siege X** overhaul — audio rework, advanced rappel, destructible props",
            "Modernized maps — Bank, Border, Chalet, Clubhouse and Kafe Dostoyevsky",
            "6v6 **Dual Front** mode with attack and defense at once",
            "Neutral electricity slows instead of damaging, limb damage reduced",
        ],
    },
    Entry {
        key: "Y10S3_HighStakes",
        release: "Sep 2, 2025",
        operators: &[Operator {
            name: "Denari",
            gadget_name: "T.R.I.P. Connector",
            gadget_desc: "creates laser tripwires that slow and injure enemies",
            img: Some("denari-og"),
        }],
        maps: &[],
        highlights: &[
            "Modernized maps — Consulate, Nighthaven Labs and Lair",
            "Blackbeard nerf, no magnified scopes on defender automatic weapons",
            "Reaper MK2 machine pistol for Sledge, Oryx, Pulse, Ying and Rook",
            "**Keres Safe Room** data extraction objective for Dual Front",
        ],
    },
    Entry {
        key: "Y10S4_TenfoldPursuit",
        release: "Dec 2, 2025",
        operators: &[Operator {
            name: "Thatcher",
            gadget_name: "E.G.S. Disruptor",
            gadget_desc: "disables all electronics in a targeted area",
            img: None,
        }],
        maps: &[Map {
            name: "Fortress",
            img: None,
        }],
        highlights: &[
            "PMR90A2 marksman rifle for Thatcher, Hibana, Capitão and Nøkk",
            "Modernized maps — Skyscraper and Theme Park",
            "Mute only jams wireless signals",
            "Hard breachers rebalanced — Ace, Hibana, Thermite and Maverick",
        ],
    },
    Entry {
        key: "Y11S1_SilentHunt",
        release: "Mar 3, 2026",
        operators: &[Operator {
            name: "Solid Snake",
            gadget_name: "Soliton Radar MKIII",
            gadget_desc: "handheld radar that marks nearby hostiles on a minimap",
            img: None,
        }],
        maps: &[],
        highlights: &[
            "Modernized maps — Coastline, Villa and Oregon",
            "Shield operators can no longer push through intact barricades",
            "Faster Skopós with less health and quicker Shell swaps",
        ],
    },
    Entry {
        key: "Y11S2_SystemOverride",
        release: "Jun 2, 2026",
        operators: &[Operator {
            name: "Dokkaebi",
            gadget_name: "Jegeo Payload",
            gadget_desc: "hacks one defender phone to detonate and cut their cameras",
            img: None,
        }],
        maps: &[Map {
            name: "Calypso Casino",
            img: None,
        }],
        highlights: &[
            "Modernized maps — Emerald Plains, Kanal and Outback",
            "XK23 assault rifle for Dokkaebi, Rauora and Sens",
            "Gridlock makes enemies limp with Trax Stingers",
            "Zofia faster again with less health",
        ],
    },
    Entry {
        key: "Y11S3_SplitFire",
        release: "Sep 1, 2026",
        operators: &[Operator {
            name: "Noor",
            gadget_name: "Horus Lance Launcher",
            gadget_desc: "fires lances that pierce shields and walls, then burst into flames",
            img: None,
        }],
        maps: &[],
        highlights: &[
            "Modernized maps — Stadium Alpha, Stadium Bravo and House",
            "Villa rework — basement bomb site and an indoor garage",
            "Jegeo Payload needs a constant link, Breach Charges for Dokkaebi",
            "Kali rebalanced — CSRX 300 with 3.5x and 8x scopes and a faster bolt",
        ],
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gpui::AssetSource;

    use super::*;
    use crate::assets::Assets;

    fn pictures() -> BTreeSet<String> {
        let of = |entry: &Entry| {
            let operators = entry.operators.iter().map(Operator::picture);
            operators.chain(entry.maps.iter().map(Map::picture))
        };
        SEASON_INFO.iter().flat_map(of).collect()
    }

    #[test]
    fn every_season_of_the_manifest_has_an_entry() {
        let downloads = backend::manifest::downloads();
        assert_eq!(SEASON_INFO.len(), downloads.len());
        for download in downloads {
            assert!(entry(&download.key).is_some(), "{}", download.key);
        }
    }

    #[test]
    fn every_picture_an_entry_refers_to_is_embedded() {
        for picture in pictures() {
            assert!(Assets.load(&picture).unwrap().is_some(), "{picture}");
        }
    }

    #[test]
    fn every_embedded_picture_is_referred_to() {
        let referred = pictures();
        for picture in Assets.list("info/").unwrap() {
            assert!(referred.contains(picture.as_ref()), "{picture}");
        }
    }
}
