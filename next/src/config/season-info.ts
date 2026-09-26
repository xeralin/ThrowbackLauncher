import type { ReactNode } from "react";
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
  note?: ReactNode;
  setup?: ReactNode;
};

const SLOW_CLOSE_NOTE =
  "Closing this season from the in-game menu can take up to 10 seconds.";

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
      "Spectator camera on all platforms",
      "Flashbang range and effectiveness roughly doubled",
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
      "Weapon charms, skins and operator headgear customization",
      "New weapons — MPX, SPAS-12, Mk17 CQB, SR-25 and D-50",
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
      "Angled grip attachment for faster ADS transitions",
      "Surrender vote system for Ranked matches",
      "**Tactical Realism** custom game mode with minimal HUD",
    ],
    note: SLOW_CLOSE_NOTE,
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
      "Destruction rework — improved wall physics, bullet holes scaled by caliber",
    ],
    note: SLOW_CLOSE_NOTE,
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
    highlights: ["New weapons — C7E, PDW9 and Vector .45 ACP"],
    note: SLOW_CLOSE_NOTE,
  },
  Y2S2_Health: {
    release: "Jun 7, 2017",
    operators: [],
    maps: [],
    highlights: [
      "Maintenance season with no new operators or maps",
      "Hitboxes limited to the operator body, improved hit registration and servers",
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
      "Sweeping texture, lighting and sky dome overhaul",
      "Extensive weapon, gadget and operator balance tweaks",
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
      "Buff and debuff icons at the screen edge",
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
      "Three-player PvE against infected enemies",
      "Reload rework with ADS cancelling and resume points",
    ],
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
      "**Pick & Ban** with two attacker and two defender bans per team",
      "Going prone from standing now breaks ADS, countering dropshots",
      "New defuser animation using a tool",
      "Second Yokai drone for Echo and Clubhouse map buffs",
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
      "Consulate buff with a fourth bomb site and revised window lines",
    ],
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
      "Main menu rework with clearer navigation",
      "SMG-11 secondary for Mute, slower weapon swap for Clash",
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
      "**Newcomer** playlist for players under level 50",
      "MMR rollback refunding rank changes from cheater matches",
      "Preset bomb sites and a 3:30 action phase in Casual",
    ],
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
      "**Pick & Ban** added to Ranked",
      "Reverse friendly fire extended to all damage types",
    ],
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
      "**Unranked** playlist with the full Ranked ruleset",
      "**Champion** rank above Diamond at 5000+ MMR",
    ],
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
      "CSRX 300 — the first bolt-action sniper rifle",
      "Limb penetration added for most weapons",
      "Manual confirmation required for rappel exits",
    ],
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
      "Attacker drone spawns made deterministic instead of random",
      "Barricade debris cleanup for consistent sightlines",
    ],
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
      "Unified global MMR replacing region-specific ranked ratings",
    ],
  },
  Y5S3_ShadowLegacy: {
    release: "Sep 10, 2020",
    operators: [
      {
        name: "Zero",
        gadgetName: "ARGUS Launcher",
        gadgetDesc: "launches cameras that pierce walls and fire laser shots",
      },
    ],
    maps: [{ name: "Chalet", img: "chalet-y5s3" }],
    highlights: [
      "**Ping 2.0** contextual pinging, usable from cams and after death",
      "**Hard Breach Charge** secondary gadget for attackers",
      "Map ban voting before matches",
      "Optics overhaul with new 1.5x and 2.0x scopes",
    ],
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
      "Yokai drone for Echo made permanently visible",
      "Runout detection timer cut from 2s to 1s",
    ],
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
      "Gonne-6 explosive secondary added to select attacker loadouts",
      "**Match Replay** to re-watch matches from any angle",
      "**Newcomer** playlist reworked with a rotating seasonal map",
    ],
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
      "Smoke gas propagation rework stops gas passing through surfaces",
      "Melee now shatters Black Mirrors, Evil Eyes and Bulletproof Cameras",
    ],
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
      "Armor stat converted to health points",
      "Individual attacker spawn selection in all playlists",
      "Suppressor damage penalty unified at 15% on primary weapons",
    ],
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
      "UZK50Gi .50-cal SMG for Thorn",
      "HUD rework with drone counter",
      "Customizable team colors",
      "Finka can trigger Adrenal Surge while downed to revive herself",
    ],
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
      "Attacker repick during the prep phase",
      "**Team Deathmatch** added as a permanent playlist",
      "All non-magnifying sights unlocked on most weapons",
    ],
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
      "**Shooting Range** with recoil and damage lanes for weapon testing",
      "**Privacy Mode** and reputation penalties for reverse friendly fire",
      "POF-9 assault rifle for Sens",
    ],
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
      "Recoil system overhaul with progressive recoil",
      "Impact EMP grenade secondary gadget for 8 operators",
      "Rook armor plates grant **Withstand** when downed",
    ],
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
      "**Ranked 2.0** with Rank Points and new Emerald rank",
      "Crossplay between consoles and cross-progression on all platforms",
    ],
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
      "Reload rework that keeps the chambered round",
      "Playlists reorganized into Competitive, Quick Play and Training",
      "**MouseTrap** anti-cheat on consoles",
      "Operator specialties system with beginner challenges",
    ],
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
      "**Observation Blocker** secondary gadget blocks drone line of sight",
      "**Arcade** playlist made permanent with new Free For All mode",
      "Free camera added to **Match Replay**",
    ],
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
      "**Quick Match 2.0** and new Standard playlist replace Unranked",
      "Shotgun overhaul and Grim Kawan Hive buff",
      "**Weapon Roulette** permanent arcade mode",
      "Commendation system for positive player behavior",
    ],
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
      "Cooking removed from Frag grenades",
      "**Versus AI** playlist and new Map Training playlist",
      "Controller remapping and deadzone customization",
    ],
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
      "Full shield rework — sprinting, free look, guard break, no hip fire",
      "Attachment overhaul with Horizontal Grip and reworked scope zooms",
      ".44 Vendetta magnum for Deimos",
    ],
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
      "**Endless Drill** warm-up playlist with respawning enemies",
      "Major nerfs for Fenrir and Solis",
    ],
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
      "**Siege Cup** — 5v5 tournament ladder",
      "PCX-33 assault rifle for Skopós",
      "Drone speed boost",
      "DX12 as the default graphics API",
    ],
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
      "Crossplay between console and PC with separate ranked progression",
    ],
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
      "Full **Reputation System** rollout with penalties and rewards",
      "**Dynamic Matchmaking** 1.0 adapting to server load",
    ],
  },
  Y10S2_DayBreak: {
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
      "Permanent 6v6 **Dual Front** mode",
      "Modernized maps — Bank, Border, Chalet, Clubhouse and Kafe Dostoyevsky",
      "Free Access model and new **Pick & Ban** phase",
    ],
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
      "Keres Safe Room data extraction objective for **Dual Front**",
      "Blackbeard nerf, magnified sights removed from defender automatic weapons",
      "Reaper MK2 secondary weapon for select operators",
      "Modernized maps — Consulate, Nighthaven Labs and Lair",
    ],
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
      "Ranked matchmaking factoring visible rank alongside hidden MMR",
      "PMR90A2 marksman rifle for Thatcher, Hibana, Capitão and Nøkk",
      "**Wildcards Siege** 10th anniversary event on House",
    ],
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
      "Major balancing update targeting entry fraggers and roamers",
      "Ranked map pool reduced from 16 to 13 maps",
      "TACIT .45 suppressed secondary pistol",
      "Modernized maps — Coastline, Villa and Oregon",
    ],
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
      "**Ranked 3.0** — revamped and more transparent competitive system",
      "Modernized maps — Emerald Plains, Kanal and Outback",
      "XK23 assault rifle for Dokkaebi, Rauora and Sens",
    ],
  },
};

export const HM_INFO: Omit<SeasonInfoEntry, "release"> = {
  operators: [],
  maps: [],
  highlights: [
    `Full R6S **SDK** by [DataCluster0](${site.heatedMetalRepoUrl}) for specific old builds`,
    "**Quarrel** scripting language and in-game map editor",
    "Cosmetic and attachment unlocks without restrictions",
    "In-game console, weapon inspection and custom keybinds",
  ],
};
