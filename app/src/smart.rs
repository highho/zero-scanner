use crate::model::{parse_smart_value, ScanType, VType};

#[derive(Clone, Debug)]
pub struct SmartPlan {
    pub title: String,
    pub reason: String,
    pub vtype: VType,
    pub scan_type: ScanType,
    pub value: String,
    pub confidence: u8,
    pub steps: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct Preset {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub vtype: VType,
    pub value: &'static str,
}

pub const PRESETS: [Preset; 4] = [
    Preset {
        key: "health",
        label: "生命值",
        hint: "受伤后通常会减少",
        vtype: VType::I32,
        value: "100",
    },
    Preset {
        key: "ammo",
        label: "弹药",
        hint: "射击后通常会减少",
        vtype: VType::I32,
        value: "30",
    },
    Preset {
        key: "coins",
        label: "金币",
        hint: "拾取后通常会增加",
        vtype: VType::I32,
        value: "250",
    },
    Preset {
        key: "coords",
        label: "坐标",
        hint: "移动时会连续变化，尝试浮点",
        vtype: VType::F32,
        value: "0",
    },
];


// Game-profile presets registered by plugins. Kept separate from the built-in
// PRESETS so the built-in list stays a const and the UI can label the two
// sources differently.
static PROFILE_PRESETS: std::sync::OnceLock<std::sync::Mutex<Vec<Preset>>> =
    std::sync::OnceLock::new();

fn profile_presets() -> &'static std::sync::Mutex<Vec<Preset>> {
    PROFILE_PRESETS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Register a preset that came from a plugin game profile.
///
/// Presets use static strings so the scan page can keep copying them around;
/// registration happens once at startup, so the leak is bounded and small.
pub fn register_profile_preset(preset: Preset) -> Result<(), String> {
    if preset.label.trim().is_empty() || preset.value.trim().is_empty() {
        return Err(format!("游戏配置「{}」缺少标签或数值。", preset.key));
    }
    let mut guard = profile_presets().lock().unwrap();
    if guard.iter().any(|p| p.key.eq_ignore_ascii_case(preset.key)) {
        return Err(format!("游戏配置「{}」已经注册过了。", preset.key));
    }
    guard.push(preset);
    Ok(())
}

/// Every preset the scan page should offer: built-ins first, then plugins.
pub fn all_presets() -> Vec<Preset> {
    let mut list: Vec<Preset> = PRESETS.to_vec();
    list.extend(profile_presets().lock().unwrap().iter().copied());
    list
}

/// Presets contributed by plugins only.
pub fn plugin_presets() -> Vec<Preset> {
    profile_presets().lock().unwrap().clone()
}

/// Drop plugin presets. Used by the self-test to stay isolated.
pub fn clear_profile_presets() {
    profile_presets().lock().unwrap().clear();
}
fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// A deliberately local, rule-based interpreter: it never sends text anywhere.
pub fn interpret(text: &str) -> Option<SmartPlan> {
    let lower = text.to_ascii_lowercase();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let found = parse_smart_value(trimmed);
    let value = found.as_ref().map(|(raw, _)| raw.clone());
    let decimal = found
        .as_ref()
        .map(|(raw, _)| raw.contains('.'))
        .unwrap_or(false);

    let health = contains_any(&lower, &["血量", "生命", "血", "health", "hp"]);
    let ammo = contains_any(&lower, &["弹药", "子弹", "ammo", "magazine"]);
    let money = contains_any(
        &lower,
        &["金币", "金钱", "钱", "余额", "硬币", "coin", "gold", "money", "cash"],
    );
    let speed = contains_any(&lower, &["速度", "speed", "velocity"]);
    let position = contains_any(
        &lower,
        &["坐标", "位置", "position", "coord", "x轴", "y轴", "z轴"],
    );
    let mana = contains_any(&lower, &["法力", "蓝量", "蓝", "mana", "mp"]);
    let level = contains_any(&lower, &["等级", "经验", "level", "experience", "exp"]);
    let damage = contains_any(&lower, &["伤害", "攻击", "damage", "attack"]);

    let (title, vtype, reason, advice) = if health {
        (
            "查找生命值",
            if decimal { VType::F32 } else { VType::I32 },
            "生命值通常是一个 4 字节整数（少数游戏用浮点）。",
            "回到游戏受伤，让血量减少，再用「数值减少」再次扫描。",
        )
    } else if ammo {
        (
            "查找弹药",
            VType::I32,
            "弹药通常是 4 字节整数。",
            "射出一发子弹，再用「数值减少」再次扫描。",
        )
    } else if money {
        (
            "查找金币 / 余额",
            VType::I32,
            "金币和余额通常是 4 字节整数；金额很大时可以换 8 字节。",
            "拾取或花费金币，再用「数值增加 / 减少」再次扫描。",
        )
    } else if speed {
        (
            "查找速度",
            VType::F32,
            "游戏速度通常是浮点数。",
            "移动或改变速度后，用「数值变化」再次扫描。",
        )
    } else if position {
        (
            "查找坐标",
            VType::F32,
            "坐标通常由多个连续浮点组成。",
            "移动角色后，用「数值变化」再次扫描。",
        )
    } else if mana {
        (
            "查找法力值",
            if decimal { VType::F32 } else { VType::I32 },
            "法力值通常是 4 字节整数。",
            "施放技能后，用「数值减少」再次扫描。",
        )
    } else if level || damage {
        (
            "查找成长数值",
            VType::I32,
            "等级、经验或伤害通常是 4 字节整数。",
            "让它变化一次，再用「数值增加 / 减少」再次扫描。",
        )
    } else if contains_any(&lower, &["未知", "不知道", "initial", "unknown"]) {
        (
            "从未知初值开始",
            VType::I32,
            "没有明确的目标值时，可以先拍一张内存快照，再筛选变化。",
            "让目标数值变化一次，再用「数值变化 / 减少」再次扫描。",
        )
    } else {
        (
            "尝试精确扫描",
            if decimal { VType::F32 } else { VType::I32 },
            "没有识别到具体的游戏术语，先按最常见的 4 字节整数处理。",
            "如果结果很多，回到游戏改变数值，再用「数值变化」缩小范围。",
        )
    };

    let (scan_type, value) = if let Some(value) = value {
        (ScanType::Exact, value)
    } else if contains_any(&lower, &["未知", "不知道", "initial", "unknown"]) {
        (ScanType::Unknown, String::new())
    } else {
        return None;
    };

    let confidence = if found.is_some() && (health || ammo || money || speed || position || mana)
    {
        92
    } else if found.is_some() {
        72
    } else {
        55
    };

    let steps = vec![
        format!("选择 {} 类型，数值填 {}", vtype.label(), if value.is_empty() { "—".to_string() } else { value.clone() }),
        "确认目标进程后执行首次扫描".to_string(),
        advice.to_string(),
    ];

    Some(SmartPlan {
        title: title.to_string(),
        reason: reason.to_string(),
        vtype,
        scan_type,
        value,
        confidence,
        steps,
    })
}

pub fn state_steps(has_process: bool, hits: usize, entries: usize) -> (usize, Vec<String>) {
    if !has_process {
        (
            0,
            vec![
                "选择一个正在运行的游戏进程".to_string(),
                "输入你在游戏里看到的数值".to_string(),
                "让数值变化，再次扫描并锁定".to_string(),
            ],
        )
    } else if hits == 0 {
        (
            1,
            vec![
                "进程已连接，可以开始扫描".to_string(),
                "输入游戏里当前的数值并首次扫描".to_string(),
                "回到游戏让数值变化，再用相对扫描缩小范围".to_string(),
            ],
        )
    } else if hits <= 8 {
        (
            2,
            vec![
                format!("只剩 {hits} 个候选地址"),
                "把结果加入地址列表并修改/锁定".to_string(),
                "如果数值没有变化，回到游戏触发一次变化".to_string(),
            ],
        )
    } else {
        (
            2,
            vec![
                format!("还有 {hits} 个候选地址"),
                "回到游戏让数值变化，使用「数值变化 / 减少 / 增加」".to_string(),
                if entries > 0 {
                    "也可以先锁定最可疑的地址观察".to_string()
                } else {
                    "重复缩小范围，直到只剩少数地址".to_string()
                },
            ],
        )
    }
}
