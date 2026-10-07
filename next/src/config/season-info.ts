import { site } from "@/config/site";

export type InfoOperator = {
  name: string;
  gadgetName: string;
  gadgetDesc: string;
  img?: string;
};

export type InfoMap = {
  name: string;
  img?: string;
};

export type SeasonInfoEntry = {
  release: string;
  operators: InfoOperator[];
  maps: InfoMap[];
  highlights: string[];
  events?: string[];
  requirements?: (string | false)[];
  slowClose?: boolean;
};

export const SEASON_INFO: Record<string, SeasonInfoEntry> = {
  Y1S0_Vanilla: {
    release: "Dec 1, 2015",
    operators: [],
    maps: [],
    highlights: [],
  },
  Y1S1_BlackIce: {
    release: "Feb 2, 2016",
    operators: [
      {
        name: "Buck",
        gadgetName: "Skeleton Key",
        gadgetDesc: "adds an under-barrel 12 gauge shotgun for breaching",
      },
      {
        name: "Frost",
        gadgetName: "Welcome Mat",
        gadgetDesc: "floor trap that downs attackers who step on it",
      },
    ],
    maps: [{ name: "Yacht" }],
    highlights: [
      "Option for attackers to pick their own spawn location",
      "Stun Grenade effect range roughly doubled",
      "Spectator Camera with keyboard and mouse controls",
    ],
  },
  Y1S2_DustLine: {
    release: "May 10, 2016",
    operators: [
      {
        name: "Blackbeard",
        gadgetName: "TARS Rifle Shield",
        gadgetDesc: "transparent ballistic shield mounted on his rifle",
        img: "blackbeard-og",
      },
      {
        name: "Valkyrie",
        gadgetName: "Black Eye",
        gadgetDesc: "throwable sticky camera that gives defenders a live feed",
      },
    ],
    maps: [{ name: "Border", img: "border-y1s2" }],
    highlights: [
      "Loadout changes between rounds",
      "Montagne gains side protection on his shield",
      "Stun Grenade detonates 1s after impact",
    ],
  },
  Y1S3_SkullRain: {
    release: "Aug 2, 2016",
    operators: [
      {
        name: "Capitão",
        gadgetName: "Tactical Crossbow",
        gadgetDesc: "fires silent asphyxiating bolts and micro smoke grenades",
      },
      {
        name: "Caveira",
        gadgetName: "Silent Step",
        gadgetDesc: "makes her movement nearly silent while active",
      },
    ],
    maps: [{ name: "Favela", img: "favela-y1s3" }],
    highlights: [
      "**Claymore** for attackers, **Impact Grenade** for defenders",
      "Twitch, Doc and Blackbeard reworked",
      "Angled Grip attachment for faster ADS transitions",
      "**Tactical Realism** custom game mode with minimal HUD",
    ],
    slowClose: true,
  },
  Y1S4_RedCrow: {
    release: "Nov 17, 2016",
    operators: [
      {
        name: "Hibana",
        gadgetName: "X-KAIROS",
        gadgetDesc: "launches explosive pellets that breach reinforced walls",
        img: "hibana-og",
      },
      {
        name: "Echo",
        gadgetName: "Yokai",
        gadgetDesc:
          "ceiling-clinging drone that fires disorienting ultrasonic bursts",
      },
    ],
    maps: [{ name: "Skyscraper", img: "skyscraper-y1s4" }],
    highlights: [
      "Caliber-based destruction — stronger rounds make bigger holes",
      "Neck shots count as headshots, limbs take full damage",
      "ACOG removed from the SMG-11",
    ],
    slowClose: true,
  },
  Y2S1_VelvetShell: {
    release: "Feb 7, 2017",
    operators: [
      {
        name: "Jackal",
        gadgetName: "Eyenox Model III",
        gadgetDesc: "scans enemy footprints to track and ping their location",
      },
      {
        name: "Mira",
        gadgetName: "Black Mirror",
        gadgetDesc:
          "bulletproof, ejectable one-way mirror for reinforced or soft walls",
      },
    ],
    maps: [{ name: "Coastline", img: "coastline-y2s1" }],
    highlights: [
      "Sight positions harmonized across all weapons",
      "Capitão gets a Claymore, IQ gets Frag Grenades",
    ],
    slowClose: true,
  },
  Y2S2_Health: {
    release: "Jun 7, 2017",
    operators: [],
    maps: [],
    highlights: [
      "Maintenance season with no new operators or maps",
      "Hitboxes cover only the body, not gear or headgear",
      "Ash and Thermite get Stun Grenades, Fuze and Jackal Smoke Grenades",
      "Glaz gets a weaker OTs-03 after his thermal scope rework",
    ],
  },
  Y2S3_BloodOrchid: {
    release: "Sep 5, 2017",
    operators: [
      {
        name: "Ying",
        gadgetName: "Candela",
        gadgetDesc: "releases a cluster of flash charges to blind enemies",
      },
      {
        name: "Lesion",
        gadgetName: "Gu Mine",
        gadgetDesc: "injects a toxin that damages and slows enemies",
      },
      {
        name: "Ela",
        gadgetName: "Grzmot Mine",
        gadgetDesc: "proximity concussion mine that dazes and impairs hearing",
      },
    ],
    maps: [{ name: "Theme Park", img: "theme-park-y2s3" }],
    highlights: [
      "Attackers can deploy both drones at the same time",
      "ACOG removed from 2- and 3-speed defenders",
      "Smoke Grenades made opaque, cut to 2 per operator",
      "Lighting and sky rework on all maps, with less bloom",
    ],
  },
  Y2S4_WhiteNoise: {
    release: "Dec 5, 2017",
    operators: [
      {
        name: "Dokkaebi",
        gadgetName: "Logic Bomb",
        gadgetDesc: "hacks defender phones to ring and reveal their positions",
        img: "dokkaebi-og",
      },
      {
        name: "Vigil",
        gadgetName: "ERC-7",
        gadgetDesc: "wipes his image from any cameras in view",
      },
      {
        name: "Zofia",
        gadgetName: "KS79 Lifeline",
        gadgetDesc: "fires concussion and impact grenades from a launcher",
      },
    ],
    maps: [{ name: "Tower" }],
    highlights: [
      "New pistol recoil animation and a higher fire rate",
      "Thrown grenades inherit player movement",
      "Color-coded HUD icons for buffs and debuffs",
    ],
  },
  Y3S1_Chimera: {
    release: "Mar 6, 2018",
    operators: [
      {
        name: "Lion",
        gadgetName: "EE-ONE-D",
        gadgetDesc: "aerial drone scans and pings moving enemies through walls",
      },
      {
        name: "Finka",
        gadgetName: "Adrenal Surge",
        gadgetDesc: "boosts team health and revives downed allies",
      },
    ],
    maps: [],
    highlights: [
      "Reload rework — ADS interrupt and resume points",
      "**Raw Input** option for mouse and keyboard",
      "Blitz 2-speed/2-armor, Ela gets a weaker Scorpion",
    ],
    events: ["Outbreak"],
  },
  Y3S2_ParaBellum: {
    release: "Jun 7, 2018",
    operators: [
      {
        name: "Alibi",
        gadgetName: "Prisma",
        gadgetDesc:
          "deploys holograms that ping enemies who shoot or touch them",
      },
      {
        name: "Maestro",
        gadgetName: "Evil Eye",
        gadgetDesc: "deploys bulletproof remote cameras that fire laser bursts",
      },
    ],
    maps: [{ name: "Villa", img: "villa-y3s2" }],
    highlights: [
      "**Bulletproof Camera** secondary gadget for defenders",
      "**Pick & Ban** in custom games, up to 4 operator bans",
      "Going prone from standing breaks ADS, countering dropshots",
      "Second Yokai drone for Echo, Clubhouse map buff",
    ],
  },
  Y3S3_GrimSky: {
    release: "Sep 4, 2018",
    operators: [
      {
        name: "Maverick",
        gadgetName: "Breaching Torch",
        gadgetDesc:
          "silently burns small holes through reinforced walls and hatches",
      },
      {
        name: "Clash",
        gadgetName: "CCE Shield",
        gadgetDesc: "extendable shield that tasers enemies to slow them down",
        img: "clash-og",
      },
    ],
    maps: [{ name: "Hereford Base" }],
    highlights: [
      "Sight misalignment fixes and hatch destruction rework",
      "Consulate buff with a fourth bomb site and safer spawns",
      "Thatcher disables cameras only temporarily",
    ],
    events: ["Mad House"],
  },
  Y3S4_WindBastion: {
    release: "Dec 4, 2018",
    operators: [
      {
        name: "Nomad",
        gadgetName: "Airjab Launcher",
        gadgetDesc:
          "launches repulsion grenades that knock back nearby enemies",
      },
      {
        name: "Kaid",
        gadgetName: "Rtila Electroclaw",
        gadgetDesc: "electrifies reinforced walls, hatches and barbed wire",
      },
    ],
    maps: [{ name: "Fortress", img: "fortress-y3s4" }],
    highlights: [
      "New throw curves for all throwables except the Nitro Cell",
      "SMG-11 machine pistol for Mute",
    ],
  },
  Y4S1_BurntHorizon: {
    release: "Mar 6, 2019",
    operators: [
      {
        name: "Gridlock",
        gadgetName: "Trax Stingers",
        gadgetDesc:
          "deploys spreading spike traps that slow and damage enemies",
      },
      {
        name: "Mozzie",
        gadgetName: "Pest Launcher",
        gadgetDesc: "launches Pests that hack and steal attacker drones",
      },
    ],
    maps: [{ name: "Outback", img: "outback-y4s1" }],
    highlights: [
      "Revived operators get 20 HP instead of 50",
      "Lean camera moved to the center of the head",
      "Faster, safer Breach Charges",
      "Mute blocks calls from Dokkaebi",
    ],
    events: ["Rainbow is Magic"],
  },
  Y4S2_PhantomSight: {
    release: "Jun 11, 2019",
    operators: [
      {
        name: "Nøkk",
        gadgetName: "HEL Presence Reduction",
        gadgetDesc:
          "hides her from observation tools and muffles her footsteps",
      },
      {
        name: "Warden",
        gadgetName: "Glance Smart Glasses",
        gadgetDesc: "grants vision through smoke and protection from flashes",
      },
    ],
    maps: [{ name: "Kafe Dostoyevsky", img: "kafe-dostoyevsky-y4s2" }],
    highlights: [
      "Glaz must stand still to see through smoke",
      "Cameras and drones light up in team colors",
      "HUD stays visible when flashed or stunned",
      "Rounds can no longer end in a draw",
    ],
    events: ["Showdown"],
  },
  Y4S3_EmberRise: {
    release: "Sep 11, 2019",
    operators: [
      {
        name: "Amaru",
        gadgetName: "Garra Hook",
        gadgetDesc:
          "grapples to ledges, windows and open hatches for fast entry",
      },
      {
        name: "Goyo",
        gadgetName: "Volcán Shield",
        gadgetDesc: "deployable shield with an incendiary charge on the back",
      },
    ],
    maps: [{ name: "Kanal" }],
    highlights: [
      "Secondary gadgets swapped for 11 operators",
      "Redesigned Deployable Shield fits doorframes",
      "Shield ADS time raised from 0.4s to 0.6s",
      "Terrorist Hunt without bombers or trap rooms",
    ],
    events: ["Doktor's Curse", "Money Heist"],
  },
  Y4S4_ShiftingTides: {
    release: "Dec 3, 2019",
    operators: [
      {
        name: "Kali",
        gadgetName: "LV Explosive Lance",
        gadgetDesc:
          "fires a lance that destroys gadgets on both sides of walls",
      },
      {
        name: "Wamai",
        gadgetName: "Mag-NET System",
        gadgetDesc: "attracts enemy projectiles and detonates them near itself",
      },
    ],
    maps: [{ name: "Theme Park", img: "theme-park-y4s4" }],
    highlights: [
      "Limb penetration for most weapons",
      "Rappel exits need manual confirmation",
      "Jackal gets pings based on footprint age",
    ],
    events: ["Road To S.I. 2020"],
  },
  Y5S1_VoidEdge: {
    release: "Mar 10, 2020",
    operators: [
      {
        name: "Iana",
        gadgetName: "Gemini Replicator",
        gadgetDesc: "projects a remote-controlled holographic clone of Iana",
      },
      {
        name: "Oryx",
        gadgetName: "Remah Dash",
        gadgetDesc: "dashes to knock down enemies and smash through soft walls",
      },
    ],
    maps: [{ name: "Oregon", img: "oregon-y5s1" }],
    highlights: [
      "Explosions deal shrapnel damage that cover blocks",
      "Drones spawn on the chosen spawn side",
      "Twitch, Lesion and Warden reworked",
      "Barricade debris cleanup for consistent sightlines",
    ],
    events: ["Grand Larceny", "Golden Gun"],
  },
  Y5S2_SteelWave: {
    release: "Jun 16, 2020",
    operators: [
      {
        name: "Ace",
        gadgetName: "S.E.L.M.A. Aqua Breacher",
        gadgetDesc: "uses hydraulic pressure to breach reinforced surfaces",
      },
      {
        name: "Melusi",
        gadgetName: "Banshee Sonic Defense",
        gadgetDesc:
          "emits noise and slows attackers in range and line of sight",
      },
    ],
    maps: [{ name: "House" }],
    highlights: [
      "**Proximity Alarm** secondary gadget for defenders",
      "Amaru breaks through hatches with the Garra Hook",
      "Concussion no longer rolls the camera or alters sensitivity",
      "ACS12 shotgun switched to slugs",
    ],
    events: ["M.U.T.E. Protocol"],
  },
  Y5S3_ShadowLegacy: {
    release: "Sep 10, 2020",
    operators: [
      {
        name: "Zero",
        gadgetName: "ARGUS Launcher",
        gadgetDesc: "launches cameras that pierce walls and fire laser shots",
      },
      {
        name: "Tachanka",
        gadgetName: "Shumikha Launcher",
        gadgetDesc: "launches incendiary grenades that ignite after bouncing",
      },
    ],
    maps: [{ name: "Chalet", img: "chalet-y5s3" }],
    highlights: [
      "Defenders share a **Reinforcement Pool** of 10",
      "**Hard Breach Charge** secondary gadget for attackers",
      "**Ping 2.0** contextual pings, also on cams and after death",
      "DP27 light machine gun for Tachanka",
    ],
    events: ["Sugar Fright"],
  },
  Y5S4_NeonDawn: {
    release: "Dec 1, 2020",
    operators: [
      {
        name: "Aruni",
        gadgetName: "Surya Gate",
        gadgetDesc:
          "deploys a laser gate that damages attackers passing through",
      },
      {
        name: "Jäger",
        gadgetName: "Active Defense System",
        gadgetDesc: "unlimited projectile intercepts on a 10 second cooldown",
      },
    ],
    maps: [{ name: "Skyscraper", img: "skyscraper-y5s4" }],
    highlights: [
      "Runout detection timer cut from 2s to 1s",
      "Echo can no longer cloak his Yokai drones",
      "Hibana can fire 2, 4 or 6 X-KAIROS pellets at once",
      "Reinforcements deploy in 4.5s instead of 5.5s",
    ],
    events: ["Legacy", "Road To S.I. 2021"],
  },
  Y6S1_CrimsonHeist: {
    release: "Mar 16, 2021",
    operators: [
      {
        name: "Flores",
        gadgetName: "RCE-Ratero Charge",
        gadgetDesc: "remote-controlled explosive drone that destroys gadgets",
      },
    ],
    maps: [{ name: "Border", img: "border-y6s1" }],
    highlights: [
      "GONNE-6 explosive hand cannon for 8 attackers",
      "**Match Replay** to re-watch matches from any angle",
      "Mute also disables Claymores and Airjabs",
      "Tactical Realism removed from custom games",
    ],
    events: ["Rainbow is Magic", "Apocalypse"],
  },
  Y6S2_NorthStar: {
    release: "Jun 14, 2021",
    operators: [
      {
        name: "Thunderbird",
        gadgetName: "Kóna Station",
        gadgetDesc: "deployable station that heals or revives nearby operators",
      },
    ],
    maps: [{ name: "Favela" }],
    highlights: [
      "Single bullet holes in walls no longer give line of sight",
      "Melee shatters Black Mirrors, Evil Eyes and Bulletproof Cameras",
      "Gas from Smoke no longer spreads through walls and floors",
      "Bodies of eliminated operators replaced by icons",
    ],
    events: ["Containment"],
  },
  Y6S3_CrystalGuard: {
    release: "Sep 7, 2021",
    operators: [
      {
        name: "Osa",
        gadgetName: "Talon-8 Clear Shield",
        gadgetDesc: "transparent bulletproof shield she carries or deploys",
      },
    ],
    maps: [
      { name: "Bank", img: "bank-y6s3" },
      { name: "Coastline", img: "coastline-y2s1" },
      { name: "Clubhouse", img: "clubhouse-y6s3" },
    ],
    highlights: [
      "Armor converted to health — 100, 110 or 125 HP",
      "Twitch gets a jumping Shock Drone that destroys gadgets",
      "Linear damage drop-off, flat 15% suppressor penalty",
      "Attackers always pick their own spawn location",
    ],
    events: ["Showdown", "Doktor's Curse"],
  },
  Y6S4_HighCalibre: {
    release: "Nov 30, 2021",
    operators: [
      {
        name: "Thorn",
        gadgetName: "Razorbloom Shell",
        gadgetDesc: "sticks to surfaces and bursts lethal blades near enemies",
      },
    ],
    maps: [{ name: "Outback" }],
    highlights: [
      "Bulletproof Camera rotates and fires EMP bursts",
      "HUD rework, drone counter and customizable team colors",
      "Finka can revive herself with Adrenal Surge",
    ],
    events: ["Snow Brawl", "Road To S.I. 2022"],
  },
  Y7S1_DemonVeil: {
    release: "Mar 15, 2022",
    operators: [
      {
        name: "Azami",
        gadgetName: "Kiba Barrier",
        gadgetDesc: "thrown kunai expands into a bulletproof barrier",
      },
      {
        name: "Goyo",
        gadgetName: "Volcán Canister",
        gadgetDesc: "incendiary canister that ignites when shot",
      },
    ],
    maps: [{ name: "Emerald Plains" }],
    highlights: [
      "**Attacker Repick** during the prep phase",
      "**Team Deathmatch** mode, playable in custom games",
      "All non-magnifying sights available on most weapons",
      "Outdoor defender cameras lose signal after 10s",
    ],
    events: ["Rengoku"],
  },
  Y7S2_VectorGlare: {
    release: "Jun 14, 2022",
    operators: [
      {
        name: "Sens",
        gadgetName: "R.O.U. Projector System",
        gadgetDesc:
          "rolls and projects a light wall that blocks lines of sight",
      },
    ],
    maps: [{ name: "Close Quarter" }],
    highlights: [
      "**Shooting Range** with recoil and damage lanes",
      "Third secondary weapon option for 9 operators",
    ],
    events: ["M.U.T.E. Protocol"],
  },
  Y7S3_BrutalSwarm: {
    release: "Sep 6, 2022",
    operators: [
      {
        name: "Grim",
        gadgetName: "Kawan Hive Launcher",
        gadgetDesc: "launches bot swarms that reveal enemies passing through",
      },
    ],
    maps: [{ name: "Stadium Bravo", img: "stadium-bravo-y7s3" }],
    highlights: [
      "Recoil rework — progressive recoil",
      "**Impact EMP Grenade** secondary gadget for attackers",
      "Suppressors no longer reduce damage, more attachment options",
      "Rook armor plates grant Withstand when downed",
    ],
    events: ["Snipers", "Doktor's Curse"],
  },
  Y7S4_SolarRaid: {
    release: "Dec 6, 2022",
    operators: [
      {
        name: "Solis",
        gadgetName: "SPEC-IO Electro-Sensor",
        gadgetDesc: "detects and marks enemy electronic devices",
      },
    ],
    maps: [{ name: "Nighthaven Labs", img: "nighthaven-labs-y7s4" }],
    highlights: [
      "All operators move at the same speed while aiming",
      "New health and speed ratings for 13 operators",
      "Extended Barrel adds 15% weapon damage",
      "No prep-phase friendly fire, optional in custom games",
    ],
    events: ["Snow Brawl"],
  },
  Y8S1_CommandingForce: {
    release: "Mar 7, 2023",
    operators: [
      {
        name: "Brava",
        gadgetName: "Kludge Drone",
        gadgetDesc:
          "takes over enemy devices or destroys them if uncontrollable",
      },
    ],
    maps: [],
    highlights: [
      "Reload rework — interrupted reloads leave no magazine",
      "Zero triggers the ARGUS pierce manually",
      "Compensator and Muzzle Brake recoil reduction buffed",
    ],
    events: ["Rainbow is Magic"],
  },
  Y8S2_DreadFactor: {
    release: "May 30, 2023",
    operators: [
      {
        name: "Fenrir",
        gadgetName: "F-NATT Dread Mine",
        gadgetDesc: "releases fear gas that severely limits enemy vision",
      },
    ],
    maps: [{ name: "Consulate", img: "consulate-y8s2" }],
    highlights: [
      "**Observation Blocker** secondary gadget for defenders",
      "Arcade modes and the new **Free For All** in custom games",
      "Shooting Range aiming lane with moving targets",
      "Host button to randomize teams in custom games",
    ],
    events: ["Rengoku"],
  },
  Y8S3_HeavyMettle: {
    release: "Aug 29, 2023",
    operators: [
      {
        name: "Ram",
        gadgetName: "BU-GI Auto-Breacher",
        gadgetDesc: "mini-tank that destroys breakable surfaces in its path",
      },
      {
        name: "Frost",
        gadgetName: "Welcome Mat",
        gadgetDesc:
          "floor trap that downs attackers, placeable under barbed wire",
      },
    ],
    maps: [],
    highlights: [
      "Shotgun rework and bouncing Kawan Hives for Grim",
      "Lesion rework — visible Gu Mines with impact damage",
      "**Weapon Roulette** arcade mode",
      "Free camera and hideable HUD for spectators",
    ],
    events: ["Doktor's Curse"],
  },
  Y8S4_DeepFreeze: {
    release: "Dec 6, 2023",
    operators: [
      {
        name: "Tubarão",
        gadgetName: "Zoto Canister",
        gadgetDesc: "throwable canister that freezes devices and slows enemies",
      },
    ],
    maps: [{ name: "Lair", img: "lair-y8s4" }],
    highlights: [
      "Frag Grenades lose cooking, added to 5 more attackers",
      "**Versus AI** and **Map Training** replace Situations and Training Grounds",
      "Up to 4 spectators per match, switchable from the lobby",
      "Controller remapping and deadzone customization",
    ],
    events: ["Freeze For All"],
  },
  Y9S1_DeadlyOmen: {
    release: "Mar 12, 2024",
    operators: [
      {
        name: "Deimos",
        gadgetName: "DeathMARK Tracker",
        gadgetDesc: "probe seeks a marked enemy and reveals their location",
      },
    ],
    maps: [],
    highlights: [
      "Shield rework — sprint, free look, suppressive fire, no hip fire",
      "Slower ADS for all weapons",
      "Attachment rework — Horizontal Grip and new scope zooms",
    ],
    events: ["Containment"],
  },
  Y9S2_NewBlood: {
    release: "Jun 11, 2024",
    operators: [
      {
        name: "Striker",
        gadgetName: "Gadget Kit",
        gadgetDesc: "lets Striker carry two attacker secondary gadgets",
      },
      {
        name: "Sentry",
        gadgetName: "Gadget Kit",
        gadgetDesc: "equips two different defender secondary gadgets",
      },
    ],
    maps: [],
    highlights: [
      "Fenrir and Solis nerfed — fewer mines, SPEC-IO off in prep",
      "Barbed Wire deals 5 HP/s to attackers moving in it",
      "Stadium Alpha and Bravo updated, bulletproof glass removed",
      "**Endless Drill** warm-up mode with respawning enemies",
    ],
    events: ["M.U.T.E. Protocol"],
  },
  Y9S3_TwinShells: {
    release: "Sep 10, 2024",
    operators: [
      {
        name: "Skopós",
        gadgetName: "V10 Pantheon Shells",
        gadgetDesc: "swaps control between two robotic shells at will",
      },
    ],
    maps: [],
    highlights: [
      "**Drone Boost** for faster drone movement",
      "DX12 as the default graphics API, Vulkan removed",
      "1v1 custom game presets — **Short Match** and **Long Match**",
      "**Versus AI 2.0** — defend against AI attackers",
    ],
    events: ["Doktor's Curse"],
  },
  Y9S4_CollisionPoint: {
    release: "Dec 3, 2024",
    operators: [
      {
        name: "Blackbeard",
        gadgetName: "H.U.L.L. Adaptable Shield",
        gadgetDesc: "deployable shield he raises to block incoming fire",
      },
    ],
    maps: [],
    highlights: [
      "Shields nerfed — melee damage removed, earlier suppressive fire",
      "Stun Grenades reduced to 2 per operator",
      "Sens can toggle R.O.U. walls, which block thermal sights",
    ],
    events: ["Freeze For All", "Assault on Hereford"],
  },
  Y10S1_PrepPhase: {
    release: "Mar 4, 2025",
    operators: [
      {
        name: "Rauora",
        gadgetName: "D.O.M. Panel Launcher",
        gadgetDesc: "launches bulletproof panels onto doorways from a distance",
      },
    ],
    maps: [],
    highlights: [
      "DX12 as the only graphics API, DX11 removed",
      "Map Training adds Presidential Plane, Yacht and Favela",
    ],
  },
  Y10S2_Daybreak: {
    release: "Jun 10, 2025",
    operators: [
      {
        name: "Clash",
        gadgetName: "CCE Shield MK2",
        gadgetDesc:
          "electrified shield that slows attackers, anchorable in place",
      },
    ],
    maps: [{ name: "District" }],
    highlights: [
      "**Siege X** overhaul — audio rework, advanced rappel, destructible props",
      "Modernized maps — Bank, Border, Chalet, Clubhouse and Kafe Dostoyevsky",
      "6v6 **Dual Front** mode with attack and defense at once",
      "Neutral electricity slows instead of damaging, limb damage reduced",
    ],
    events: ["Showdown", "Rengoku"],
  },
  Y10S3_HighStakes: {
    release: "Sep 2, 2025",
    operators: [
      {
        name: "Denari",
        gadgetName: "T.R.I.P. Connector",
        gadgetDesc: "creates laser tripwires that slow and injure enemies",
        img: "denari-og",
      },
    ],
    maps: [],
    highlights: [
      "Modernized maps — Consulate, Nighthaven Labs and Lair",
      "Blackbeard nerf, no magnified scopes on defender automatic weapons",
      "Reaper MK2 machine pistol for Sledge, Oryx, Pulse, Ying and Rook",
      "**Keres Safe Room** data extraction objective for Dual Front",
    ],
    events: ["M.U.T.E. Protocol", "Doktor's Curse"],
  },
  Y10S4_TenfoldPursuit: {
    release: "Dec 2, 2025",
    operators: [
      {
        name: "Thatcher",
        gadgetName: "E.G.S. Disruptor",
        gadgetDesc: "disables all electronics in a targeted area",
      },
    ],
    maps: [{ name: "Fortress" }],
    highlights: [
      "PMR90A2 marksman rifle for Thatcher, Hibana, Capitão and Nøkk",
      "Modernized maps — Skyscraper and Theme Park",
      "Mute only jams wireless signals",
      "Hard breachers rebalanced — Ace, Hibana, Thermite and Maverick",
    ],
    events: ["Freeze For All"],
  },
  Y11S1_SilentHunt: {
    release: "Mar 3, 2026",
    operators: [
      {
        name: "Solid Snake",
        gadgetName: "Soliton Radar MKIII",
        gadgetDesc: "handheld radar that marks nearby hostiles on a minimap",
      },
    ],
    maps: [],
    highlights: [
      "Modernized maps — Coastline, Villa and Oregon",
      "Shield operators can no longer push through intact barricades",
      "Skopós made 3-speed/1-health with faster Shell swaps",
    ],
    events: ["Rainbow is Magic"],
  },
  Y11S2_SystemOverride: {
    release: "Jun 2, 2026",
    operators: [
      {
        name: "Dokkaebi",
        gadgetName: "Jegeo Payload",
        gadgetDesc:
          "hacks one defender phone to detonate and cut their cameras",
      },
    ],
    maps: [{ name: "Calypso Casino" }],
    highlights: [
      "Modernized maps — Emerald Plains, Kanal and Outback",
      "XK23 assault rifle for Dokkaebi, Rauora and Sens",
      "Gridlock makes enemies limp with Trax Stingers",
      "Zofia back to 2-speed/2-health",
    ],
    events: ["Rengoku"],
  },
};

export function hmInfo(beta: boolean): Omit<SeasonInfoEntry, "release"> {
  return {
    operators: [],
    maps: [],
    highlights: [
      `R6S **SDK** by [DataCluster0](${site.heatedMetalRepoUrl})`,
      `**${beta ? "Lua" : "Quarrel"}** scripting language and in-game map editor`,
      "Unrestricted **Unlock All** and weapon inspection",
    ],
  };
}
