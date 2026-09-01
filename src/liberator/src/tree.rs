use crate::tables::*;

#[derive(Clone)]
pub struct TNode {
    pub text: String,
    pub id: String,
    pub children: Vec<TNode>,
}

impl TNode {
    pub fn new() -> TNode {
        TNode {
            text: String::new(),
            id: String::new(),
            children: Vec::new(),
        }
    }
}

pub fn get_path_mut<'a>(node: &'a mut TNode, path: &[usize]) -> Option<&'a mut TNode> {
    let mut cur = node;
    for &i in path {
        cur = cur.children.get_mut(i)?;
    }
    Some(cur)
}

pub fn set_text(node: &mut TNode, path: &[usize], t: &str) {
    if let Some(n) = get_path_mut(node, path) {
        n.text = t.to_string();
    }
}

pub fn rm(node: &mut TNode, path: &[usize], idx: usize) {
    if let Some(n) = get_path_mut(node, path) {
        if idx < n.children.len() {
            n.children.remove(idx);
        }
    }
}

pub fn get_path<'a>(node: &'a TNode, path: &[usize]) -> Option<&'a TNode> {
    let mut cur = node;
    for &i in path {
        cur = cur.children.get(i)?;
    }
    Some(cur)
}

fn lookup(table: &[(&str, &str)], raw: &str) -> String {
    for (r, d) in table {
        if *r == raw {
            return d.to_string();
        }
    }
    raw.to_string()
}

pub fn map_display(raw: &str) -> String {
    lookup(MAP_NAMES, raw)
}

pub fn gametype_display(raw: &str) -> String {
    lookup(GAMETYPE_NAMES, raw)
}

pub fn json_escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub fn event_node_names(event_mode: &str) -> &'static [&'static str] {
    match event_mode {
        "Mad_House" => &["Mad House"],
        "Rainbow_Is_Magic" => &["Rainbow is Magic"],
        "Showdown" => &["Showdown"],
        "Doktors_Curse_MoneyHeist" => &["Money Heist", "Doktor's Curse"],
        "Stadium" => &["Road To S.I. 2020"],
        "Outbreak" => &["Outbreak"],
        _ => &[],
    }
}

pub fn group_event_nodes(root: &mut TNode, event_mode: &str) {
    let names = event_node_names(event_mode);
    if names.is_empty() || root.children.is_empty() {
        return;
    }
    let mut events = TNode::new();
    events.text = "Events".to_string();
    for name in names {
        if let Some(i) = root.children[0]
            .children
            .iter()
            .position(|k| k.text == *name)
        {
            events.children.push(root.children[0].children.remove(i));
        } else if let Some(i) = root.children.iter().position(|k| k.text == *name) {
            events.children.push(root.children.remove(i));
        }
    }
    if !events.children.is_empty() {
        root.children.push(events);
    }
}

pub fn group_development_nodes(root: &mut TNode, gym_index: i32, video_review_index: i32) {
    let pick = |i: i32| -> Option<usize> {
        if i >= 0 && (i as usize) < root.children.len() {
            Some(i as usize)
        } else {
            None
        }
    };
    let gym = pick(gym_index);
    let video_review = pick(video_review_index);
    match (gym, video_review) {
        (Some(g), Some(v)) => {
            let first = g.min(v);
            let second = g.max(v);
            let mut development = TNode::new();
            development.text = "Development".to_string();
            let tail = root.children.remove(second);
            let head = root.children.remove(first);
            development.children.push(head);
            development.children.push(tail);
            root.children.insert(first, development);
        }
        (Some(i), None) | (None, Some(i)) => {
            root.children[i].text = "Development".to_string();
        }
        (None, None) => {}
    }
}

pub fn tn_json(node: &TNode, out: &mut String) {
    out.push_str("{\"text\":");
    json_escape(&node.text, out);
    out.push_str(",\"id\":");
    json_escape(&node.id, out);
    out.push_str(",\"children\":[");
    out.push_str(&tn_list(&node.children));
    out.push_str("]}");
}

pub fn tn_list(nodes: &[TNode]) -> String {
    let mut out = String::new();
    for (i, k) in nodes.iter().enumerate() {
        if i != 0 {
            out.push(',');
        }
        tn_json(k, &mut out);
    }
    out
}

pub fn sort_gametypes_by_name(nodes: &mut [TNode]) {
    nodes.sort_by(|a, b| a.text.cmp(&b.text));
}

pub fn label_multiplayer(node: &mut TNode, season: i32) {
    node.text = "Multiplayer".to_string();
    set_text(node, &[0], "Hostage");
    set_text(node, &[1], "Secure Area");
    set_text(node, &[2], "Bomb");
    if season >= SEASON_Y1S2 {
        set_text(node, &[3], "Warmup");
    }
    if season >= SEASON_Y2S1 {
        set_text(node, &[4], "Canister");
    }
    for i in 0..node.children.len() {
        let mut j = 0;
        while j < node.children[i].children.len() {
            let disp = map_display(&node.children[i].children[j].text);
            node.children[i].children[j].text = disp;
            if let Some(k0) = node.children[i].children[j].children.get_mut(0) {
                k0.text = "Day".to_string();
            }
            if node.children[i].children[j].children.len() > 1 {
                node.children[i].children[j].children[1].text = "Night".to_string();
            }
            let rmv = {
                let t = &node.children[i].children[j].text;
                t == "!!FLOYD" || t == "!!Staduim Playlist" || t == "!!Western"
            };
            if rmv {
                node.children[i].children.remove(j);
                continue;
            }
            j += 1;
        }
        if season == SEASON_Y1S2 && node.children[i].children.len() > 13 {
            node.children[i].children.remove(13);
        }
        if season == SEASON_Y2S3 && node.children[i].children.len() > 17 {
            node.children[i].children.remove(17);
        }
    }
    sort_gametypes_by_name(&mut node.children);
}

pub fn label_terrorist_hunt(node: &mut TNode, season: i32) {
    node.text = "Terrorist Hunt".to_string();
    set_text(node, &[0], "Normal");
    set_text(node, &[1], "Hard");
    set_text(node, &[2], "Realistic");
    for i in 0..node.children.len() {
        for j in 0..node.children[i].children.len() {
            set_text(&mut node.children[i].children[j], &[0], "Hostage");
            set_text(&mut node.children[i].children[j], &[1], "Disarm Bomb");
            set_text(&mut node.children[i].children[j], &[2], "Elimination");
            let disp = map_display(&node.children[i].children[j].text);
            node.children[i].children[j].text = disp;
            for k in 0..node.children[i].children[j].children.len() {
                for m in 0..node.children[i].children[j].children[k].children.len() {
                    let d = gametype_display(
                        &node.children[i].children[j].children[k].children[m].text,
                    );
                    node.children[i].children[j].children[k].children[m].text = d;
                }
            }
        }
        if season == SEASON_Y1S2 && node.children[i].children.len() > 13 {
            node.children[i].children.remove(13);
        }
        if season == SEASON_Y2S3 && node.children[i].children.len() > 17 {
            node.children[i].children.remove(17);
        }
        if season == SEASON_Y3S2 {
            set_text(&mut node.children[i], &[18], "Villa");
        }
        if season >= SEASON_Y3S3 {
            set_text(&mut node.children[i], &[18], "Hereford Base - Rework");
            set_text(&mut node.children[i], &[17], "Villa");
        }
    }
    for difficulty in node.children.iter_mut() {
        for map in difficulty.children.iter_mut() {
            sort_gametypes_by_name(&mut map.children);
        }
    }
}

pub fn label_situation(node: &mut TNode, advanced_order: i32) {
    node.text = "Situations".to_string();
    set_text(node, &[0], "01 CQB Basics");
    set_text(node, &[1], "02 Suburban Extraction");
    if advanced_order != 0 {
        set_text(node, &[2], "03 Tubular Assault");
        set_text(node, &[3], "04 Asset Protection");
        set_text(node, &[4], "05 Improvise Defense");
        set_text(node, &[5], "06 No Intel");
        set_text(node, &[6], "07 Cold Zero");
        set_text(node, &[7], "08 High Value Target");
        set_text(node, &[8], "09 Neutralize Cell");
    } else {
        set_text(node, &[2], "03 High Value Target");
        set_text(node, &[3], "04 Tubular Assault");
        set_text(node, &[4], "05 Cold Zero");
        set_text(node, &[5], "06 Asset Protection");
        set_text(node, &[6], "07 Neutralize Cell");
        set_text(node, &[7], "08 No Intel");
        set_text(node, &[8], "09 Improvise Defense");
    }
    set_text(node, &[9], "10 Heavily Fortified");
    set_text(node, &[10], "Article 5");
    for i in 0..node.children.len() {
        if !node.children[i].children.is_empty() {
            set_text(&mut node.children[i], &[0], "Normal");
            set_text(&mut node.children[i], &[1], "Hard");
            set_text(&mut node.children[i], &[2], "Realistic");
        }
    }
}

pub fn label_matchmaking(node: &mut TNode) {
    node.text = "Matchmaking".to_string();
    set_text(node, &[0], "Casual");
    set_text(node, &[0, 0], "Hostage");
    set_text(node, &[0, 1], "Bomb");
    set_text(node, &[0, 2], "Secure Area");
    set_text(node, &[1], "Ranked");
    set_text(node, &[1, 0], "Hostage");
    set_text(node, &[1, 1], "Bomb");
    set_text(node, &[1, 2], "Secure Area");
    if node.children.len() == 3 {
        set_text(node, &[2], "Unranked");
        set_text(node, &[2, 0], "Bomb");
    }
}

fn relabel_lighting(node: &mut TNode) {
    if node.text == "Official Day" {
        node.text = "Day".to_string();
    } else if node.text == "Official Night" {
        node.text = "Night".to_string();
    }
    for child in &mut node.children {
        relabel_lighting(child);
    }
}

pub fn label_gym(node: &mut TNode, season: i32) {
    node.text = "Gym".to_string();
    set_text(node, &[0], "Day");
    set_text(node, &[1], "Night");
    for i in 0..node.children.len() {
        if node.children[i].children.len() == 12 {
            for j in 0..node.children[i].children.len() {
                let d = map_display(&node.children[i].children[j].text);
                node.children[i].children[j].text = d;
            }
            continue;
        }
        let d = map_display(&node.children[i].text);
        node.children[i].text = d;
    }
    if season == SEASON_Y1S2 {
        rm(node, &[], 15);
    }
    if season >= SEASON_Y2S3 {
        rm(node, &[], 0);
        rm(node, &[], 0);
    }
    relabel_lighting(node);
}

pub fn label_video_review(node: &mut TNode) {
    node.text = "Video Review".to_string();
    for i in 0..node.children.len() {
        set_text(&mut node.children[i], &[0], "House - Benchmark");
        for j in 0..node.children[i].children.len() {
            let d = map_display(&node.children[i].children[j].text);
            node.children[i].children[j].text = d;
        }
    }
}

pub fn label_outbreak(node: &mut TNode) {
    node.text = "Outbreak".to_string();
    set_text(node, &[0], "Missions");
    set_text(node, &[0, 0], "Sierra Paradise");
    set_text(node, &[0, 1], "Sierra Paradise 2");
    set_text(node, &[0, 2], "Sierra Veterans Wing");
    set_text(node, &[0, 3], "Sierra Veterans Wing 2");
    set_text(node, &[0, 4], "The Nest");
    set_text(node, &[0, 5], "The Nest 2");
    rm(node, &[1], 0);
    rm(node, &[1], 0);
    rm(node, &[1], 0);
    rm(node, &[1], 0);
    set_text(node, &[1, 0], "Art Review");
    set_text(node, &[1, 0, 0], "Sierra Paradise");
    set_text(node, &[1, 0, 1], "Sierra Paradise 2");
    set_text(node, &[1, 0, 2], "Sierra Veterans Wing");
    set_text(node, &[1, 0, 3], "Sierra Veterans Wing 2");
    set_text(node, &[1, 0, 4], "The Nest");
    set_text(node, &[1, 0, 5], "The Nest 2");
    rm(node, &[], 2);
    if node.children.len() > 1 {
        if node.children[1].children.len() == 1 {
            let art_review = node.children[1].children.remove(0);
            node.children[1] = art_review;
        } else {
            node.children[1].text = "Development".to_string();
        }
    }
}

pub fn tag_ids(node: &mut TNode, tag: &str) {
    if !node.id.is_empty() {
        node.id = format!("{}{}", tag, node.id);
    }
    for k in node.children.iter_mut() {
        tag_ids(k, tag);
    }
}

pub fn add_original_hereford(node: &mut TNode) {
    let mut i = 0;
    while i < node.children.len() {
        if node.children[i].text == "Hereford Base - Rework" {
            let mut clone = node.children[i].clone();
            clone.text = "Hereford Base".to_string();
            tag_ids(&mut clone, "hereford1:");
            tag_ids(&mut node.children[i], "hereford0:");
            node.children.insert(i + 1, clone);
            i += 2;
            continue;
        }
        add_original_hereford(&mut node.children[i]);
        i += 1;
    }
}
