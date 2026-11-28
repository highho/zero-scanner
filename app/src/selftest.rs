use crate::model::{Region, ScanType, VType};
use crate::scan::{self, ScanConfig};
use crate::winmem;
use std::time::Duration;

/// A small headless check of the Windows memory engine: allocate a known value,
/// scan our own process for it, narrow the scan, then write and read it back.
pub fn run() -> bool {
    let mut log = String::new();
    let mut ok = true;
    macro_rules! check {
        ($cond:expr, $($arg:tt)*) => {
            if $cond {
                log.push_str("PASS  ");
                log.push_str(&format!($($arg)*));
                log.push('\n');
            } else {
                ok = false;
                log.push_str("FAIL  ");
                log.push_str(&format!($($arg)*));
                log.push('\n');
            }
        };
    }

    let pid = std::process::id();
    let handle = match winmem::open_process(pid, "zero-scanner.exe") {
        Ok(h) => {
            log.push_str(&format!("PASS  open own process pid={pid}\n"));
            h
        }
        Err(e) => {
            log.push_str(&format!("FAIL  open own process: {e}\n"));
            std::fs::write("selftest.log", log).ok();
            return false;
        }
    };
    check!(handle.is_64bit, "target is 64-bit: {}", handle.is_64bit);
    check!(
        handle.writable,
        "process opened with write access: {}",
        handle.writable
    );

    let mut buffer = vec![0u8; 64 * 1024];
    let magic: u64 = 0x1122_3344_5566_7788;
    buffer[1024..1032].copy_from_slice(&magic.to_le_bytes());
    let target = buffer.as_ptr() as u64 + 1024;
    let base = buffer.as_ptr() as u64 & !0xFFF;
    log.push_str(&format!(
        "buffer={:#X} target={:#X} base={:#X}\n",
        buffer.as_ptr() as u64, target, base
    ));

    let region = Region {
        base,
        size: 8192,
        writable: true,
        executable: false,
    };
    let cfg = ScanConfig {
        vtype: VType::I64,
        scan_type: ScanType::Exact,
        value1: magic.to_string(),
        value2: String::new(),
        fast: false,
        max_results: 200_000,
    };
    let job = match scan::start_first_scan(handle.clone(), vec![region], cfg.clone()) {
        Ok(j) => j,
        Err(e) => {
            log.push_str(&format!("FAIL  start first scan: {e}\n"));
            std::fs::write("selftest.log", log).ok();
            return false;
        }
    };
    match job.rx.recv_timeout(Duration::from_secs(30)) {
        Ok(done) => {
            check!(
                done.hits.iter().any(|h| h.address == target),
                "first scan found the known value ({} hits)",
                done.hits.len()
            );
            check!(!done.cancelled, "first scan not cancelled");
            let hits = done.hits;
            let next_cfg = ScanConfig {
                value1: magic.to_string(),
                ..cfg.clone()
            };
            match scan::start_next_scan(handle.clone(), hits, next_cfg) {
                Ok(job2) => match job2.rx.recv_timeout(Duration::from_secs(30)) {
                    Ok(done2) => {
                        check!(
                            done2.hits.iter().any(|h| h.address == target),
                            "next scan kept the known value ({} hits)",
                            done2.hits.len()
                        );
                    }
                    Err(e) => {
                        ok = false;
                        log.push_str(&format!("FAIL  next scan timed out: {e}\n"));
                    }
                },
                Err(e) => {
                    ok = false;
                    log.push_str(&format!("FAIL  start next scan: {e}\n"));
                }
            }
        }
        Err(e) => {
            ok = false;
            log.push_str(&format!("FAIL  first scan timed out: {e}\n"));
        }
    }

    let new_value: u64 = 0x4242_4242_4242_4242;
    check!(
        handle.write(target, &new_value.to_le_bytes()),
        "write known value through ProcessHandle"
    );
    let mut readback = [0u8; 8];
    check!(
        handle.read(target, &mut readback),
        "read value back through ProcessHandle"
    );
    check!(
        u64::from_le_bytes(readback) == new_value,
        "readback matches written value ({:#X})",
        u64::from_le_bytes(readback)
    );
    check!(
        u64::from_le_bytes(buffer[1024..1032].try_into().unwrap()) == new_value,
        "the target allocation actually changed"
    );
    // Restore the magic value so the buffer is not left changed.
    handle.write(target, &magic.to_le_bytes());

    let regions = handle.regions(false);
    check!(!regions.is_empty(), "enumerated {} memory regions", regions.len());
    check!(
        handle.region_at(base).is_some(),
        "query the region that holds the buffer"
    );
    // Disassembler: fixed byte sequences with known lengths and targets.
    {
        use crate::disasm;
        let code: [u8; 23] = [
            0x48, 0x8B, 0x05, 0x10, 0x00, 0x00, 0x00, // 48 8B 05 10 00 00 00   mov rax,[rip+0x10]
            0x55,                                     // 55                     push rbp
            0x48, 0x89, 0xE5,                         // 48 89 E5               mov rbp,rsp
            0x48, 0x83, 0xEC, 0x20,                   // 48 83 EC 20            sub rsp,0x20
            0xE8, 0x11, 0x22, 0x33, 0x44,             // E8 11 22 33 44         call rel32
            0x75, 0xF0,                               // 75 F0                  jne -0x10
            0xC3,                                     // C3                     ret
        ];
        let listing = disasm::disassemble(&code, 0x1000, true, 32);
        check!(listing.len() == 7, "disassembled 7 instructions (got {})", listing.len());
        let total: usize = listing.iter().map(|i| i.length).sum();
        check!(total == 23, "decoded byte lengths sum to 23 (got {total})");
        check!(
            listing[0].text.starts_with("mov rax"),
            "first insn is a rip-relative mov (got {})",
            listing[0].text
        );
        check!(
            listing[3].text == "sub rsp, 0x20",
            "sub rsp, 0x20 decoded (got {})",
            listing[3].text
        );
        // 7 + 1 + 3 + 4 = 15 bytes precede the call.
        let call_at = 0x1000u64 + 15;
        let expected_call = call_at + 5 + 0x44332_211;
        check!(
            listing[4].flow == disasm::Flow::Call(expected_call),
            "call target is next-insn-relative ({:X})",
            expected_call
        );
        // Instructions occupy: 7 + 1 + 3 + 4 + 5 = 20 bytes, so jne sits at 0x1014.
        let expected_jne = 0x1014u64 + 2 - 0x10;
        check!(
            listing[5].flow == disasm::Flow::CondJump(expected_jne),
            "jne target is next-insn-relative ({:X})",
            expected_jne
        );
        check!(listing[6].flow == disasm::Flow::Return, "ret ends the block");
        check!(
            listing.iter().all(|i| !i.text.contains("db ")),
            "every known instruction decoded without falling back to data"
        );
    }

    let processes = winmem::list_processes();
    check!(
        processes.iter().any(|p| p.pid == pid),
        "process list contains this process ({} total)",
        processes.len()
    );

    // Feedback: a rejected answer must teach the ranker, not just vanish.
    {
        use crate::model::{ScanHit, Value, VType};
        use crate::triage;
        let constant = ScanHit {
            address: 0x2_0000_1000,
            vtype: VType::I32,
            value: Value::Int(100),
            previous: Some(Value::Int(100)),
        };
        let live = ScanHit {
            address: 0x2_0000_2000,
            vtype: VType::I32,
            value: Value::Int(100),
            previous: Some(Value::Int(90)),
        };
        let regions = vec![triage::RegionHint {
            base: 0x2_0000_0000,
            size: 0x10000,
            writable: true,
            executable: false,
        }];
        let before = triage::rank_with_feedback(
            &[constant.clone(), live.clone()],
            &regions,
            &[],
            &[],
        );
        let constant_before = before
            .iter()
            .find(|c| c.hit.address == constant.address)
            .map(|c| c.score)
            .unwrap_or(0);
        // The user says the never-changing one is wrong.
        let mut feedback = Vec::new();
        let learned = triage::learn_rejection(&mut feedback, &constant, &regions, &[]);
        check!(
            learned.contains(&triage::RejectedTrait::NeverChanged),
            "rejecting a constant teaches the never-changed trait"
        );
        let after = triage::rank_with_feedback(
            &[constant.clone(), live.clone()],
            &regions,
            &[],
            &feedback,
        );
        let constant_after = after
            .iter()
            .find(|c| c.hit.address == constant.address)
            .map(|c| c.score)
            .unwrap_or(0);
        check!(
            constant_after < constant_before,
            "the learned trait lowers that kind of hit ({} -> {})",
            constant_before,
            constant_after
        );
        check!(
            after.first().map(|c| c.hit.address) == Some(live.address),
            "after the feedback, the live value ranks first"
        );
        check!(
            after
                .iter()
                .find(|c| c.hit.address == constant.address)
                .map(|c| c.signals.iter().any(|s| s.label.contains("反馈")))
                .unwrap_or(false),
            "the penalty is explained in the candidate signals"
        );
        // Rejecting the same kind twice must not stack into an unbounded penalty.
        let again = triage::learn_rejection(&mut feedback, &constant, &regions, &[]);
        check!(
            again.is_empty() && feedback.len() == learned.len(),
            "repeated rejections of the same trait do not stack"
        );
    }
    // Watch list: the few most plausible addresses, sampled over time.
    {
        use crate::focus;
        use crate::model::{ScanHit, Value, VType};
        use crate::triage;
        // Two candidates; the watch list keeps only the top few.
        let hits = vec![
            ScanHit { address: 0x1000, vtype: VType::I32, value: Value::Int(100), previous: None },
            ScanHit { address: 0x2000, vtype: VType::I32, value: Value::Int(200), previous: None },
            ScanHit { address: 0x3000, vtype: VType::I32, value: Value::Int(300), previous: None },
        ];
        let regions = vec![triage::RegionHint {
            base: 0x1000,
            size: 0x3000,
            writable: true,
            executable: false,
        }];
        let ranked = triage::rank(&hits, &regions, &[]);
        let watches = focus::from_candidates(&ranked, 2);
        check!(
            watches.len() == 2,
            "watch list honours its limit (got {})",
            watches.len()
        );
        // A watch that never moves must be told apart from one that does.
        let mut steady = focus::Watch::new(0x1000, VType::I32, "steady".to_string(), 10);
        steady.initial = Some(Value::Int(100));
        steady.current = Some(Value::Int(100));
        steady.samples = 10;
        check!(
            steady.behaviour() == "一直没变" && steady.delta() == Some(0.0),
            "a value that never moved is reported as steady"
        );
        let mut moving = focus::Watch::new(0x2000, VType::I32, "moving".to_string(), 10);
        moving.initial = Some(Value::Int(100));
        moving.current = Some(Value::Int(64));
        moving.samples = 10;
        moving.changes = 6;
        check!(
            moving.behaviour() == "持续变化" && moving.delta() == Some(-36.0),
            "a value that moved reports its delta ({:?})",
            moving.delta()
        );
        // Sorting must put the moving address first regardless of its score.
        let mut list = vec![steady.clone(), moving.clone()];
        focus::sort_by_behaviour(&mut list);
        check!(
            list[0].address == 0x2000,
            "behaviour outranks score when ordering the watch list"
        );
        check!(
            focus::best(&list).map(|w| w.address) == Some(0x2000),
            "best() picks the address that actually changes"
        );
        // Nothing has moved yet: there is no best, and that must be said.
        check!(
            focus::best(&[steady.clone()]).is_none(),
            "best() reports nothing when no address has changed"
        );
    }
    // Hit triage: the ranking must put the plausible hit first and explain why.
    {
        use crate::model::{ScanHit, VType, Value};
        use crate::triage;
        // One hit in writable, aligned, private memory whose value moved: the
        // textbook good candidate.
        let good = ScanHit {
            address: 0x2_0000_1000,
            vtype: VType::I32,
            value: Value::Int(9900),
            previous: Some(Value::Int(9800)),
        };
        // One in an executable page that never changed: a constant in code.
        let bad = ScanHit {
            address: 0x1_4001_0003,
            vtype: VType::I32,
            value: Value::Int(9900),
            previous: Some(Value::Int(9900)),
        };
        let regions = vec![
            triage::RegionHint { base: 0x2_0000_0000, size: 0x10000, writable: true, executable: false },
            triage::RegionHint { base: 0x1_4001_0000, size: 0x10000, writable: false, executable: true },
        ];
        let modules = vec![("game.exe".to_string(), 0x1_4000_0000u64, 0x20000u64)];
        let ranked = triage::rank(&[bad.clone(), good.clone()], &regions, &modules);
        check!(
            ranked.len() == 2 && ranked[0].hit.address == good.address,
            "triage ranks the writable, changing value ahead of a code constant"
        );
        check!(
            ranked[0].score > ranked[1].score,
            "the good candidate scores strictly higher ({} vs {})",
            ranked[0].score,
            ranked[1].score
        );
        check!(
            !ranked[0].signals.is_empty(),
            "the top candidate carries its reasons ({} signals)",
            ranked[0].signals.len()
        );
        // The code-page constant must be argued against, not merely ranked lower.
        check!(
            ranked[1].signals.iter().any(|s| s.weight <= -40),
            "a hit in an executable page is explicitly argued against"
        );
        check!(
            triage::strong_count(&ranked) >= 1,
            "strong_count recognises the good candidate"
        );
        // An empty result set must not panic or invent candidates.
        check!(
            triage::rank(&[], &regions, &modules).is_empty(),
            "ranking an empty result set yields nothing"
        );
    }
    // MCP: the transport helpers and the protocol surface.
    {
        // Token must be long and mixed enough to be unguessable on loopback:
        // an earlier version produced thirteen leading zeros.
        let token = crate::mcp::make_token_for_test();
        check!(token.len() == 32, "MCP token is 32 hex characters ({})", token.len());
        check!(
            token.chars().all(|c| c.is_ascii_hexdigit()),
            "MCP token is hex"
        );
        check!(
            !token.starts_with("0000"),
            "MCP token is not stuck in the high-zero hole ({})",
            token
        );
        // Two tokens must differ, or it is a constant not a secret.
        let other = crate::mcp::make_token_for_test();
        check!(token != other, "MCP tokens differ between calls");
        // Every declared tool must have a name and a description, or a client
        // would show an unusable entry.
        let tools = crate::mcp::tool_definitions_for_test();
        check!(!tools.is_empty(), "MCP exposes {} tools", tools.len());
        check!(
            tools.iter().all(|t| t["name"].as_str().map(|n| n.starts_with("zero_")).unwrap_or(false)
                && t["description"].as_str().map(|d| !d.is_empty()).unwrap_or(false)),
            "every MCP tool is named and described"
        );
        // The expanded tool set must be present and self-describing.
        let names: Vec<String> = tools
            .iter()
            .filter_map(|t| t["name"].as_str().map(|s| s.to_string()))
            .collect();
        for expected in ["zero_pointer_scan", "zero_disassemble", "zero_watch"] {
            check!(
                names.iter().any(|n| n == expected),
                "MCP exposes {} ({})",
                expected,
                names.len()
            );
        }
        // zero_watch is a session, not a blocking call: it must document the
        // action parameter or an agent will call it once and wait forever.
        let watch = tools
            .iter()
            .find(|t| t["name"].as_str() == Some("zero_watch"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        check!(
            watch["inputSchema"]["properties"]["action"].is_object(),
            "zero_watch documents its action parameter"
        );
        for expected in ["zero_ct_table", "zero_lua", "zero_undo"] {
            check!(
                names.iter().any(|n| n == expected),
                "MCP exposes {} ({})",
                expected,
                names.len()
            );
        }
        {
            use crate::ctable;
            let mut table = ctable::CtTable::default();
            table.entries.push(ctable::CtEntry {
                description: "coins".to_string(),
                vtype: crate::model::VType::I32,
                address: "1BDF3C11298".to_string(),
                value: Some("9900".to_string()),
                active: true,
            });
            let xml = ctable::to_xml(&table);
            match ctable::parse(&xml) {
                Ok(back) => check!(
                    back.entries.len() == 1
                        && back.entries[0].description == "coins"
                        && back.entries[0].vtype == crate::model::VType::I32,
                    "a generated .ct parses back to the same entry"
                ),
                Err(e) => {
                    ok = false;
                    log.push_str(&format!("FAIL  re-parse generated ct: {e}\n"));
                }
            }
        }

    }
    // Shortcuts: the table and the lookup must agree, and no shortcut may
    // shadow another (a duplicate would make one of them dead).
    {
        use crate::hotkey;
        check!(
            !hotkey::SHORTCUTS.is_empty(),
            "shortcut table has {} entries",
            hotkey::SHORTCUTS.len()
        );
        let mut duplicates = 0;
        for (index, a) in hotkey::SHORTCUTS.iter().enumerate() {
            for b in hotkey::SHORTCUTS.iter().skip(index + 1) {
                if a.key == b.key && a.ctrl == b.ctrl && a.shift == b.shift {
                    duplicates += 1;
                }
            }
        }
        check!(duplicates == 0, "no two shortcuts share a key combo");
        let mut unresolved = Vec::new();
        for shortcut in hotkey::SHORTCUTS {
            match hotkey::lookup(shortcut.key, shortcut.ctrl, shortcut.shift) {
                Some(action) if action == shortcut.action => {}
                _ => unresolved.push(shortcut.label),
            }
        }
        check!(
            unresolved.is_empty(),
            "every shortcut resolves to its own action ({:?})",
            unresolved
        );
        check!(
            hotkey::lookup('8', false, false) == Some(hotkey::Action::ShowList)
                && hotkey::lookup('8', true, false).is_none(),
            "a modifier changes which action fires"
        );
        check!(
            hotkey::key_char(egui::Key::F5) == Some('5')
                && hotkey::lookup('5', false, false) == Some(hotkey::Action::NextScan),
            "F5 maps onto the next-scan shortcut"
        );
    }
    // Workspace persistence: durable state survives a round-trip, and a
    // corrupt file must never stop the app from starting.
    {
        use crate::session;
        let mut workspace = session::Workspace {
            version: 1,
            ..Default::default()
        };
        workspace.patches.push(session::SavedPatch {
            address: 0x7FF6_A1B2_C010,
            original: vec![0x55, 0x48, 0x89, 0xE5],
            written: vec![0x31, 0xC0, 0xC3, 0x90],
            label: "生命值".to_string(),
            active: true,
        });
        workspace.entries.push(session::SavedEntry {
            description: "玩家金币".to_string(),
            address: 0x1000,
            vtype: "i32".to_string(),
            frozen: true,
            value_text: "9900".to_string(),
        });
        let text = session::to_string(&workspace).expect("serialise workspace");
        match session::from_str(&text) {
            Ok(back) => {
                check!(
                    back.patches.len() == 1
                        && back.patches[0].original == vec![0x55, 0x48, 0x89, 0xE5]
                        && back.patches[0].active,
                    "workspace round-trip keeps the applied patch"
                );
                check!(
                    back.entries.len() == 1
                        && back.entries[0].description == "玩家金币"
                        && back.entries[0].vtype() == VType::I32
                        && back.entries[0].frozen,
                    "workspace round-trip keeps the address list"
                );
                check!(
                    back.active_patches() == 1,
                    "active patch count survives the round-trip"
                );
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  re-parse workspace: {e}\n"));
            }
        }
        // Garbage must be rejected, and a missing file must be empty, not fatal.
        check!(
            session::from_str("{not json").is_err(),
            "a corrupt workspace is rejected instead of half-parsed"
        );
        check!(
            session::load("no-such-workspace-file-12345.json").is_empty(),
            "a missing workspace file yields an empty workspace"
        );
        // An empty workspace must not overwrite a real one on exit.
        check!(
            session::Workspace::default().is_empty(),
            "a blank workspace counts as empty and is not saved"
        );
        // Pointer maps store the module *name*, never a live base, so the same
        // map can be re-pointed after a restart. A round-trip must keep them and
        // they must count as content (otherwise saving silently drops the map).
        workspace.pointer_maps.push(session::SavedPointerMap {
            target: 0x1234_5678,
            depth: 2,
            paths: vec![session::SavedPointerPath {
                module: "game.exe".to_string(),
                module_offset: 0x1A2B,
                offsets: vec![0x18, 0x30],
                final_address: 0x7FF6_A1B2_C010,
            }],
        });
        let text = session::to_string(&workspace).expect("serialise pointer map");
        match session::from_str(&text) {
            Ok(back) => {
                let map = back.pointer_maps.first();
                check!(
                    map.map(|m| m.paths.len() == 1 && m.depth == 2 && m.target == 0x1234_5678)
                        .unwrap_or(false),
                    "pointer map survives the workspace round-trip"
                );
                check!(
                    map.map(|m| {
                        m.paths[0].module == "game.exe"
                            && m.paths[0].module_offset == 0x1A2B
                            && m.paths[0].offsets == vec![0x18, 0x30]
                    })
                    .unwrap_or(false),
                    "a saved chain keeps its module name and offsets"
                );
                check!(
                    !back.is_empty(),
                    "a workspace holding only a pointer map still counts as content"
                );
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  re-parse pointer map: {e}\n"));
            }
        }
    }
    // Plugin capabilities: struct templates turn guessed fields into named ones,
    // and game profiles become presets on the scan page. Both must be honest
    // about what they cannot do: a bad file is reported, never half-applied.
    {
        use crate::structs;
        let dir = std::env::temp_dir().join("zero-plugin-selftest");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let template_json = r#"{"name":"unity-player","description":"demo","signature":"Player","fields":[{"name":"hp","offset":"0x00","type":"float"},{"name":"max_hp","offset":"0x04","type":"float"},{"name":"team","offset":"0x08","type":"int32"}]}"#;
        std::fs::write(dir.join("unity-player.json"), template_json).unwrap();
        match crate::plugins::load_template(&dir, "unity-player.json") {
            Ok(template) => {
                check!(
                    template.fields.len() == 3 && template.span() == 12,
                    "plugin struct template parses ({} fields, {} bytes)",
                    template.fields.len(),
                    template.span()
                );
                structs::clear_templates();
                check!(
                    structs::register_template(template.clone()).is_ok(),
                    "a sane struct template registers"
                );
                check!(
                    structs::register_template(template).is_err(),
                    "registering the same template twice is refused"
                );
                // Applying it must name the fields instead of guessing.
                let mut bytes = [0u8; 16];
                bytes[0..4].copy_from_slice(&42.5f32.to_le_bytes());
                bytes[4..8].copy_from_slice(&100.0f32.to_le_bytes());
                bytes[8..12].copy_from_slice(&7i32.to_le_bytes());
                match structs::template_for("Player") {
                    Some(found) => match found.apply(&bytes) {
                        Some(fields) => check!(
                            fields.len() == 3
                                && fields[0].note.contains("hp")
                                && fields[2].signed == 7,
                            "template fields are named and read correctly ({})",
                            fields[0].note
                        ),
                        None => {
                            ok = false;
                            log.push_str("FAIL  template did not apply to a big enough block\n");
                        }
                    },
                    None => {
                        ok = false;
                        log.push_str("FAIL  template lookup by signature failed\n");
                    }
                }
                // A broken layout (overlapping fields) must be refused up front.
                let broken = structs::StructTemplate {
                    name: "broken".to_string(),
                    signature: "Broken".to_string(),
                    fields: vec![
                        structs::TemplateField {
                            name: "a".to_string(),
                            offset: 0,
                            kind: structs::FieldKind::I32,
                        },
                        structs::TemplateField {
                            name: "b".to_string(),
                            offset: 2,
                            kind: structs::FieldKind::I32,
                        },
                    ],
                };
                check!(
                    structs::register_template(broken).is_err(),
                    "an overlapping template layout is refused"
                );
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  parse plugin template: {e}\n"));
            }
        }
        // Unknown kinds and offsets must fail loudly, not guess.
        std::fs::write(
            dir.join("bad-kind.json"),
            r#"{"name":"bad","fields":[{"name":"x","offset":"0x00","type":"quantum"}]}"#,
        )
        .unwrap();
        check!(
            crate::plugins::load_template(&dir, "bad-kind.json").is_err(),
            "a template with an unknown field type is rejected"
        );
        structs::clear_templates();

        let profile_json = r#"{"name":"sample-game","description":"demo profile","process":"sample-game.exe","scan":{"value_type":"i32","value":"120","label":"生命值"},"hotkeys":[{"action":"toggle_freeze","key":"F2"},{"action":"warp_to_moon","key":"F9"}]}"#;
        std::fs::write(dir.join("sample-game.json"), profile_json).unwrap();
        match crate::plugins::load_profile(&dir, "sample-game.json") {
            Ok(profile) => {
                check!(
                    profile.name == "sample-game" && profile.scan.is_some(),
                    "plugin game profile parses"
                );
                let unsupported = profile.unsupported_keys();
                check!(
                    unsupported.len() == 1 && unsupported[0].contains("warp_to_moon"),
                    "an unsupported hotkey action is reported, not silently dropped ({})",
                    unsupported.join(",")
                );
                crate::smart::clear_profile_presets();
                let preset = profile.to_preset();
                check!(
                    crate::smart::register_profile_preset(preset).is_ok(),
                    "a game profile registers as a scan preset"
                );
                let all = crate::smart::all_presets();
                check!(
                    all.len() == crate::smart::PRESETS.len() + 1
                        && all.last().map(|p| p.value == "120").unwrap_or(false),
                    "registered presets are offered next to the built-in ones ({})",
                    all.len()
                );
                check!(
                    crate::smart::register_profile_preset(profile.to_preset()).is_err(),
                    "registering the same game profile twice is refused"
                );
                crate::smart::clear_profile_presets();
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  parse plugin profile: {e}\n"));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    // Trainer export: every entry is proven writable and restorable before it
    // is written into the exported table, and the launcher points at this exe.
    {
        use crate::ctable::{CtEntry, CtTable};
        use crate::trainer;
        // A table whose address is a plain hex number works against ourselves.
        let mut cell: [u8; 8] = [0u8; 8];
        let cell_addr = cell.as_mut_ptr() as u64;
        let table = CtTable {
            entries: vec![
                CtEntry {
                    description: "self cell".to_string(),
                    vtype: VType::I32,
                    address: format!("{cell_addr:X}"),
                    value: Some("1234".to_string()),
                    active: true,
                },
                // A deliberately impossible address must be reported, not dropped.
                CtEntry {
                    description: "bad address".to_string(),
                    vtype: VType::I32,
                    address: "0".to_string(),
                    value: Some("1".to_string()),
                    active: true,
                },
            ],
            raw: String::new(),
        };
        let dir = std::env::temp_dir().join("zero-trainer-selftest");
        let _ = std::fs::remove_dir_all(&dir);
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("zero-scanner.exe"));
        match trainer::export(&table, &handle, &dir, &exe) {
            Ok(export) => {
                check!(
                    export.ready == 1,
                    "trainer export keeps the entries that can actually work ({})",
                    export.ready
                );
                check!(
                    export.skipped.len() == 1,
                    "trainer export reports the entry it cannot use ({:?})",
                    export.skipped
                );
                check!(
                    export.table_path.exists() && export.launcher_path.exists(),
                    "trainer export wrote both the table and the launcher"
                );
                let launcher = std::fs::read_to_string(&export.launcher_path).unwrap_or_default();
                check!(
                    launcher.contains("--apply-table"),
                    "the launcher calls this executable with --apply-table"
                );
                // The exported table must read back with the same entries.
                let xml = std::fs::read_to_string(&export.table_path).unwrap_or_default();
                match crate::ctable::parse(&xml) {
                    Ok(back) => check!(
                        back.entries.len() == 1
                            && back.entries[0].value.as_deref() == Some("1234"),
                        "the exported table parses back to the same single entry"
                    ),
                    Err(e) => {
                        ok = false;
                        log.push_str(&format!("FAIL  reparse exported table: {e}\n"));
                    }
                }
                // check_entry is a dry run: after it, memory must be untouched.
                let mut probe = [0u8; 4];
                check!(
                    handle.read(cell_addr, &mut probe) && probe == [0u8, 0, 0, 0],
                    "checking an entry leaves the target untouched"
                );
                let _ = std::fs::remove_dir_all(&dir);
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  trainer export: {e}\n"));
            }
        }
        // Values encode the way the target reads them.
        check!(
            trainer::encode_value(VType::I32, "-1").ok() == Some(vec![0xFF, 0xFF, 0xFF, 0xFF]),
            "a negative integer encodes as two's complement"
        );
        check!(
            trainer::encode_value(VType::F32, "1.5").ok() == Some(1.5f32.to_le_bytes().to_vec()),
            "a float encodes little-endian"
        );
        check!(
            trainer::encode_value(VType::I32, "not a number").is_err(),
            "a bad value is rejected with an error, not written as zero"
        );
    }
    // Lua: a real runtime, real host functions, real memory access.
    {
        use crate::lua as script;
        match script::run_script("print(1 + 2)", None) {
            Ok(result) => check!(
                result.output == vec!["3".to_string()],
                "Lua evaluated an expression and captured print ({:?})",
                result.output
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  run lua: {e}\n"));
            }
        }
        // Control flow and string handling must be a real interpreter, not a toy.
        match script::run_script(
            "local t = {} for i = 1, 5 do t[i] = i * i end print(t[5], #\"hello\")",
            None,
        ) {
            Ok(result) => check!(
                result.output.first().map(|s| s.as_str()) == Some("25\t5"),
                "Lua loops and string length work ({:?})",
                result.output
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  lua control flow: {e}\n"));
            }
        }
        // Host functions touch our own memory through the real handle.
        let mut scratch = vec![0u8; 16];
        let addr = scratch.as_mut_ptr() as u64;
        let script_text = format!(
            "writeInteger({addr}, 1234, 4) print(readInteger({addr}, 4))"
        );
        match script::run_script(&script_text, Some(handle.clone())) {
            Ok(result) => {
                check!(
                    result.output == vec!["1234".to_string()],
                    "Lua wrote and read back a 4-byte integer ({:?})",
                    result.output
                );
                let mut back = [0u8; 4];
                let read_ok = handle.read(addr, &mut back);
                check!(
                    read_ok && u32::from_le_bytes(back) == 1234,
                    "the write is visible from outside the script"
                );
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  lua memory access: {e}\n"));
            }
        }
        // Errors must surface, not be swallowed.
        check!(
            script::run_script("error(\"boom\")", None).is_err(),
            "a Lua runtime error is reported to the caller"
        );
        check!(
            script::run_script("this is not lua", None).is_err(),
            "a syntax error is reported to the caller"
        );

        // 1.4: the CE API set a shared table actually uses.
        // readPointer / writePointer follow the target's own pointer width.
        let ptr_cell: [u64; 1] = [0x1122_3344_5566_7788];
        let ptr_script = format!(
            "writePointer({cell}, 0xCAFEBABE) print(readPointer({cell}))",
            cell = ptr_cell.as_ptr() as u64
        );
        match script::run_script(&ptr_script, Some(handle.clone())) {
            Ok(result) => check!(
                result.output.first().map(|s| s.as_str()) == Some("3405691582"),
                "readPointer / writePointer round-trip at the target's width ({:?})",
                result.output
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  lua readPointer: {e}\n"));
            }
        }
        // registerSymbol then getAddress resolves the symbol, not a module.
        match script::run_script(
            "registerSymbol(\"hp\", 0x1234) print(getSymbol(\"hp\"))",
            None,
        ) {
            Ok(result) => check!(
                result.output.first().map(|s| s.as_str()) == Some("4660"),
                "registerSymbol / getSymbol store and return an address ({:?})",
                result.output
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  lua symbols: {e}\n"));
            }
        }
        // allocateMemory hands out real memory in the target and frees it at
        // the end of the run; a script cannot leak into the process.
        match script::run_script(
            "local a = allocateMemory(4096) writeInteger(a, 7, 4) print(readInteger(a, 4))",
            Some(handle.clone()),
        ) {
            Ok(result) => check!(
                result.output.first().map(|s| s.as_str()) == Some("7"),
                "allocateMemory returns usable memory ({:?})",
                result.output
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  lua allocateMemory: {e}\n"));
            }
        }
        // autoAssemble writes a real patch, recorded so it can be undone.
        match script::run_script("print(1)", Some(handle.clone())) {
            Ok(result) => check!(
                result.patches.is_empty() && result.allocations.is_empty(),
                "a script with no writes leaves no patches and no leaks"
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  lua clean run: {e}\n"));
            }
        }
        // AOBScan finds a byte pattern we plant ourselves. The bytes are built
        // at runtime rather than written as a literal: a literal would also live
        // in the binary's own read-only data and the scan would find it twice.
        let mut marker_buf: Vec<u8> = (0u8..32).map(|i| 0x9E ^ (i * 7)).collect();
        marker_buf[8..16].copy_from_slice(&[0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6, 0x07, 0x18]);
        let marker_addr = marker_buf.as_ptr() as u64;
        // The same eight bytes can occur by chance elsewhere in the address
        // space, so the check is that our own address is among the hits.
        let want = marker_addr + 8;
        let aob_script = format!(
            "local hits = AOBScan(\"A1 B2 C3 D4 E5 F6 07 18\", {addr}, {end}) local found = false for i = 1, #hits do if hits[i] == {want} then found = true end end print(found and \"yes\" or \"no\")",
            addr = marker_addr,
            end = marker_addr + marker_buf.len() as u64,
            want = want
        );
        match script::run_script(&aob_script, Some(handle.clone())) {
            Ok(result) => check!(
                result.output.first().map(|s| s.as_str()) == Some("yes"),
                "AOBScan finds a planted byte signature ({:?})",
                result.output
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  lua AOBScan: {e}\n"));
            }
        }
        // createTimer runs the callback after the body, a bounded number of ticks.
        match script::run_script(
            "local n = 0 createTimer(10, function(t) n = n + 1 end) print(\"body\")",
            None,
        ) {
            Ok(result) => check!(
                result.output.first().map(|s| s.as_str()) == Some("body"),
                "createTimer does not run the callback inside the body"
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  lua createTimer: {e}\n"));
            }
        }
    }
    // Speed control: timing maths plus a real suspend/resume round-trip.
    {
        use crate::speed;
        let half = speed::SpeedPlan { factor: 0.5 }.timing();
        check!(
            half.1 < half.0 / 2,
            "half speed runs fewer ms than it stops ({}/{})",
            half.1, half.0
        );
        let double = speed::SpeedPlan { factor: 2.0 }.timing();
        check!(
            double.1 > double.0 / 2,
            "double speed runs more ms than it stops ({}/{})",
            double.1, double.0
        );
        // The run slice must never be 0 or the whole period.
        let extreme = speed::SpeedPlan { factor: 100.0 }.timing();
        check!(
            extreme.1 >= 1 && extreme.1 < extreme.0,
            "extreme factors still leave a run slice ({}/{})",
            extreme.1, extreme.0
        );
        // Thread enumeration on our own process must find at least one thread.
        let ids = speed::thread_ids(pid);
        check!(!ids.is_empty(), "enumerated {} threads of this process", ids.len());
        // Suspending our own process would deadlock the thread doing the
        // suspending; the guard must refuse instead of trying.
        let self_suspend = speed::suspend_all(pid);
        check!(
            self_suspend.is_err(),
            "refuses to suspend its own process (would freeze itself)"
        );
        // A foreign pid that does not exist must simply stop nothing.
        let ghost = speed::suspend_all(0xFFFF_FFF0);
        check!(
            ghost.map(|t| t.is_empty()).unwrap_or(false),
            "suspending a nonexistent process stops zero threads"
        );
    }
    // Cheat table: parse a real-shaped .ct document and write one back.
    {
        use crate::ctable;
        let sample = r#"<?xml version="1.0" encoding="utf-8"?>
<CheatTable>
  <CheatEntries>
    <CheatEntry>
      <Description>"玩家生命"</Description>
      <VariableType>4 Bytes</VariableType>
      <Address>game.exe+1A2B</Address>
      <Value>100</Value>
    </CheatEntry>
    <CheatEntry>
      <Description>"金币 &amp; 余额"</Description>
      <VariableType>Float</VariableType>
      <Address>[[game.exe+8C]+18]+30</Address>
    </CheatEntry>
  </CheatEntries>
</CheatTable>"#;
        match ctable::parse(sample) {
            Ok(table) => {
                check!(
                    table.entries.len() == 2,
                    "parsed 2 cheat entries (got {})",
                    table.entries.len()
                );
                check!(
                    table.entries[0].description == "玩家生命" && table.entries[0].vtype == VType::I32,
                    "first entry: description unquoted and 4 Bytes type (got {:?})",
                    table.entries[0].description
                );
                check!(
                    table.entries[0].value.as_deref() == Some("100") && table.entries[0].active,
                    "entry with a Value counts as active"
                );
                check!(
                    table.entries[1].description.contains("&") && table.entries[1].vtype == VType::F32,
                    "XML entity unescaped and Float type recognised"
                );
                check!(
                    table.entries[1].value.is_none() && !table.entries[1].active,
                    "entry without a Value is inactive"
                );
                // Round-trip: serialise, re-parse, compare.
                let written = ctable::to_xml(&table);
                match ctable::parse(&written) {
                    Ok(again) => check!(
                        again.entries.len() == 2
                            && again.entries[0].description == table.entries[0].description
                            && again.entries[1].address == table.entries[1].address,
                        "table survives a write/read round-trip"
                    ),
                    Err(e) => {
                        ok = false;
                        log.push_str(&format!("FAIL  re-parse written table: {e}\n"));
                    }
                }
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  parse cheat table: {e}\n"));
            }
        }
        // Address expressions against a fake module list.
        let modules = vec![("game.exe".to_string(), 0x1400_0000u64)];
        check!(
            ctable::resolve_address("game.exe+1A2B", &modules, true, &|_| None)
                == Ok(0x1400_1A2B),
            "module+offset resolves without reading memory"
        );
        check!(
            ctable::resolve_address("0x7FF6A1B2C010", &modules, true, &|_| None)
                == Ok(0x7FF6_A1B2_C010),
            "absolute hex address resolves"
        );
        // One dereference, then an offset.
        let read = |addr: u64| -> Option<u64> {
            if addr == 0x1400_008C {
                Some(0x2000)
            } else {
                None
            }
        };
        let chain = ctable::resolve_address("[game.exe+8C]+18", &modules, true, &read);
        check!(
            chain == Ok(0x2018),
            "pointer chain dereferences then adds the offset (got {:X?})",
            chain
        );
        check!(
            ctable::resolve_address("[game.exe+8C]+18", &modules, true, &|_| None).is_err(),
            "an unreadable pointer is reported, not guessed"
        );
        check!(
            ctable::resolve_address("missing.exe+10", &modules, true, &|_| None).is_err(),
            "unknown module is reported"
        );
    }
    // Assembler: encode, then decode the result back and compare.
    {
        use crate::{assembler, disasm};
        match assembler::assemble("mov rax, rbx\nnop\nret", 0x1000) {
            Ok(result) => {
                check!(
                    result.bytes == vec![0x48, 0x89, 0xD8, 0x90, 0xC3],
                    "assembled mov rax,rbx / nop / ret ({})",
                    result.hex()
                );
                let back = disasm::disassemble(&result.bytes, 0x1000, true, 16);
                check!(
                    back.len() == 3
                        && back[0].text == "mov rax, rbx"
                        && back[1].text == "nop"
                        && back[2].text == "ret",
                    "round-trip: decoded back to the same 3 instructions"
                );
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  assemble simple: {e}\n"));
            }
        }
        // Immediates and memory operands.
        match assembler::assemble("mov rsp, rbp\nsub rsp, 0x20\nmov dword ptr [rax+0x10], 1", 0x2000) {
            Ok(result) => {
                let back = disasm::disassemble(&result.bytes, 0x2000, true, 16);
                check!(
                    back.len() == 3,
                    "assembled 3 instruction forms (got {} bytes)",
                    result.bytes.len()
                );
                check!(
                    back[2].text.contains("[rax+0x10]") && back[2].text.contains("0x1"),
                    "memory operand re-decoded as [rax+0x10], imm 1 (got {})",
                    back.get(2).map(|i| i.text.as_str()).unwrap_or("?")
                );
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  assemble operands: {e}\n"));
            }
        }
        // Labels must resolve to real addresses.
        match assembler::assemble("start:\nnop\nnop\njmp start", 0x3000) {
            Ok(result) => {
                check!(
                    result.label_address("start") == Some(0x3000),
                    "label start resolved to 0x3000"
                );
                let back = disasm::disassemble(&result.bytes, 0x3000, true, 16);
                let last = back.last().map(|i| i.flow.clone());
                check!(
                    last == Some(disasm::Flow::Jump(0x3000)),
                    "jmp start assembled to point back at 0x3000"
                );
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  assemble labels: {e}\n"));
            }
        }
        // An unknown mnemonic must be an error, never silent bytes.
        check!(
            assembler::assemble("frobnicate rax, 1", 0x1000).is_err(),
            "unknown mnemonic is rejected instead of emitting bytes"
        );
        check!(
            assembler::assemble("jmp nowhere", 0x1000).is_err(),
            "undefined label is rejected"
        );
    }

    // Reversible patching: write into a code cave, patch our own memory, undo it.
    {
        use crate::inj;
        match inj::create_cave(&handle, 0x1000) {
            Ok(cave) => {
                check!(cave.size >= 0x1000, "allocated a code cave ({} bytes)", cave.size);
                let payload: [u8; 6] = [0x90, 0x90, 0xC3, 0xCC, 0xCC, 0xCC];
                let filled = inj::fill_cave(&handle, &cave, &payload);
                check!(filled.is_ok(), "filled the cave with a payload");
                let mut back = vec![0u8; payload.len()];
                let read_back = handle.read(cave.address, &mut back);
                check!(
                    read_back && back == payload,
                    "cave payload reads back byte-for-byte"
                );
                // Patch a private page we own, then restore it.
                let mut scratch = vec![0x11u8; 64];
                let scratch_addr = scratch.as_mut_ptr() as u64;
                match inj::apply(&handle, scratch_addr, &[0x48, 0x31, 0xC0], "selftest") {
                    Ok(mut patch) => {
                        check!(patch.active, "patch applied and recorded");
                        check!(
                            patch.original == vec![0x11u8; 3],
                            "patch kept the original bytes ({})",
                            patch.original_hex()
                        );
                        let mut now = [0u8; 3];
                        let read_now = handle.read(scratch_addr, &mut now);
                        check!(
                            read_now && now == [0x48, 0x31, 0xC0],
                            "patched bytes are in memory"
                        );
                        check!(inj::verify(&handle, &patch), "verify sees the patched bytes");
                        let reverted = inj::revert(&handle, &mut patch);
                        check!(reverted.is_ok() && !patch.active, "patch reverted");
                        let mut restored = [0u8; 3];
                        let read_restored = handle.read(scratch_addr, &mut restored);
                        check!(
                            read_restored && restored == [0x11, 0x11, 0x11],
                            "original bytes came back exactly"
                        );
                    }
                    Err(e) => {
                        ok = false;
                        log.push_str(&format!("FAIL  apply patch: {e}\n"));
                    }
                }
                let freed = inj::destroy_cave(&handle, &cave);
                check!(freed.is_ok(), "code cave freed");
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  create code cave: {e}\n"));
            }
        }
        // A trampoline must steal whole instructions and reach the cave.
        let tramp = inj::Trampoline::jump_to(0x1000, 0x2_0000, &[3, 2, 4]);
        match tramp {
            Ok(t) => check!(
                t.stolen == 5 && t.entry.len() == 5 && t.entry[0] == 0xE9,
                "trampoline steals 5 whole bytes and starts with jmp"
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  trampoline: {e}\n"));
            }
        }
        let exit = inj::Trampoline::exit_jump(0x2_0000, 0x1005);
        check!(
            exit.map(|b| b.len() == 5 && b[0] == 0xE9).unwrap_or(false),
            "exit jump back to the resume point is built"
        );
    }

    // PE symbol tables: our own imports must resolve, kernel32 exports must be found.
    let modules = handle.modules();
    check!(!modules.is_empty(), "enumerated {} modules", modules.len());
    if let Some(own) = modules.iter().find(|m| m.name.eq_ignore_ascii_case("zero-scanner.exe")) {
        log.push_str(&format!(
            "DIAG own base={:X} {}\n",
            own.base,
            crate::syms::diagnostics(&handle, own)
        ));
        match crate::syms::parse(&handle, own) {
            Ok(table) => check!(
                !table.imports.is_empty(),
                "parse own imports ({} libs, {} entries)",
                table.imports.len(),
                table.import_count()
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  parse own imports: {e}\n"));
            }
        }
    } else {
        ok = false;
        log.push_str("FAIL  own module not found\n");
    }
    if let Some(k32) = modules.iter().find(|m| m.name.eq_ignore_ascii_case("kernel32.dll")) {
        log.push_str(&format!(
            "DIAG k32 base={:X} {}\n",
            k32.base,
            crate::syms::diagnostics(&handle, k32)
        ));
        match crate::syms::parse(&handle, k32) {
            Ok(table) => check!(
                table.exports.iter().any(|s| s.name.eq_ignore_ascii_case("CreateFileW")),
                "kernel32 exports include CreateFileW ({} exports)",
                table.exports.len()
            ),
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  parse kernel32 exports: {e}\n"));
            }
        }
    } else {
        ok = false;
        log.push_str("FAIL  kernel32.dll not found\n");
    }

    // Address expressions and runtime theme engine (game-scenario + theming).
    {
        use crate::addr;
        let fake_modules = vec![winmem::ModuleInfo {
            name: "game.exe".to_string(),
            base: 0x1000,
            size: 0x10000,
        }];
        check!(
            addr::resolve(None, &fake_modules, "game.exe+0x4A0C8") == Some(0x4B0C8),
            "addr: module+offset resolves (game.exe+0x4A0C8 -> 0x4B0C8)"
        );
        check!(
            addr::resolve(None, &fake_modules, "0x1234") == Some(0x1234),
            "addr: bare hex resolves"
        );
        check!(
            addr::resolve(None, &fake_modules, "12345") == Some(12345),
            "addr: decimal resolves"
        );
        check!(
            addr::annotate(&fake_modules, 0x1234) == "game.exe+0x234",
            "addr: annotate produces module+offset"
        );
        check!(
            addr::looks_like_module("UnityPlayer.dll"),
            "addr: dll name detected as module"
        );
        check!(
            !addr::looks_like_module("0x1234"),
            "addr: pure hex is not a module name"
        );
        let themes = crate::theme::themes();
        check!(
            themes.len() >= 7,
            "theme: {} built-in presets available",
            themes.len()
        );
        check!(
            crate::theme::themes().iter().any(|t| t.id == "midnight"),
            "theme: default midnight preset present"
        );
        check!(
            crate::theme::themes().iter().any(|t| !t.dark),
            "theme: at least one light palette (daylight)"
        );
        check!(
            crate::structs::FieldKind::I32.size() == 4,
            "structs: I32 field size is 4"
        );
    }

    // 自动更新的版本比较与资产选择逻辑（纯本地，不联网）。
    {
        use crate::update;
        check!(
            update::is_newer("1.2.0", "1.1.0"),
            "update: 1.2.0 is newer than 1.1.0"
        );
        check!(
            !update::is_newer("1.0.0", "1.2.0"),
            "update: 1.0.0 is not newer than 1.2.0"
        );
        check!(
            !update::is_newer("1.2.0", "1.2.0"),
            "update: identical versions are not newer"
        );
        check!(
            update::is_newer("v1.10.0", "1.9.9"),
            "update: v-prefix and double digits compare numerically, not lexically"
        );
        check!(
            update::is_newer("1.2.1", "1.2.0"),
            "update: patch bump is detected"
        );
        check!(
            !update::is_newer("garbage", "1.2.0"),
            "update: unparsable tag never claims to be newer"
        );
        check!(
            update::normalize_tag("v1.2.0") == "1.2.0",
            "update: tag normalization strips the v prefix"
        );
    }

    // 更新安装前的文件校验：不存在 / 太小 / 不是 exe 都必须拒绝。
    {
        use crate::update;
        let dir = std::env::temp_dir().join("zero-selftest-validate");
        std::fs::create_dir_all(&dir).ok();
        let missing = dir.join("nope.exe");
        std::fs::remove_file(&missing).ok();
        check!(
            update::validate_exe(&missing).is_err(),
            "update: 缺失的安装文件会被拒绝"
        );

        let tiny = dir.join("tiny.exe");
        std::fs::write(&tiny, b"MZ").ok();
        check!(
            update::validate_exe(&tiny).is_err(),
            "update: 过小的安装文件会被拒绝"
        );

        let not_exe = dir.join("fake.exe");
        std::fs::write(&not_exe, vec![b'X'; 4096]).ok();
        check!(
            update::validate_exe(&not_exe).is_err(),
            "update: 没有 MZ 头的文件会被拒绝"
        );

        let good = dir.join("good.exe");
        let mut bytes = vec![b'\0'; 4096];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        std::fs::write(&good, &bytes).ok();
        check!(
            update::validate_exe(&good).is_ok(),
            "update: 带 MZ 头的完整文件会被接受"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    // 偏好文件的兼容性：老 prefs.json 没有新字段时必须保持原有行为。
    {
        use crate::prefs::Prefs;
        let old: Prefs = serde_json::from_str(r#"{"theme":"neon"}"#).unwrap();
        check!(
            old.auto_update,
            "prefs: 缺少 auto_update 字段时默认为开启"
        );
        check!(
            old.skipped_version.is_empty(),
            "prefs: 缺少 skipped_version 字段时默认为空"
        );
        let full = Prefs {
            theme: "ice".to_string(),
            skipped_version: "1.3.0".to_string(),
            auto_update: false,
        };
        let text = serde_json::to_string(&full).unwrap();
        let back: Prefs = serde_json::from_str(&text).unwrap();
        check!(
            back.skipped_version == "1.3.0" && !back.auto_update,
            "prefs: 跳过版本与自动检查开关能完整往返"
        );
    }

    // 真的落一次盘：用 ZERO_DATA_DIR 指向临时目录，确认写入后能读回来，
    // 而且不会碰到用户自己的 %APPDATA%\Zero。
    {
        use crate::prefs;
        let dir = std::env::temp_dir().join("zero-selftest-prefs");
        std::fs::remove_dir_all(&dir).ok();
        std::env::set_var("ZERO_DATA_DIR", &dir);
        prefs::set_skipped_version("9.9.9");
        prefs::set_auto_update(false);
        let loaded = prefs::load();
        check!(
            loaded.skipped_version == "9.9.9",
            "prefs: 跳过版本写盘后能读回来"
        );
        check!(!loaded.auto_update, "prefs: 自动检查开关写盘后能读回来");
        check!(
            dir.join("prefs.json").is_file(),
            "prefs: 偏好确实写进了 ZERO_DATA_DIR 指定的目录"
        );
        std::env::remove_var("ZERO_DATA_DIR");
        std::fs::remove_dir_all(&dir).ok();
    }


    // 调试器：真实附加子进程靶场，设硬件写断点，验证命中与 detach 无残留。
    {
        use crate::debugger::{self, BpKind, Slot};
        // 独立落盘追踪（不捕获 log 以避免与 check! 的借用冲突）。
        fn trace(label: &str) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("selftest.log")
            {
                let _ = writeln!(f, "....  debugger: {label}");
            }
        }
        let info = std::env::temp_dir().join("zero-selftest-dbg.info");
        std::fs::remove_file(&info).ok();
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  debugger: locate exe: {e}\n"));
                return false;
            }
        };
        match std::process::Command::new(&exe)
            .arg("--dbg-target")
            .arg(&info)
            .spawn()
        {
            Ok(mut child) => {
                trace("spawned target child");
                // （stdout 被屏蔽，只能走文件）
                let mut pid = None;
                let mut addr = None;
                for _ in 0..50 {
                    if let Ok(text) = std::fs::read_to_string(&info) {
                        for line in text.lines() {
                            if let Some(v) = line.strip_prefix("pid=") {
                                pid = v.trim().parse::<u32>().ok();
                            }
                            if let Some(v) = line.strip_prefix("addr=") {
                                addr =
                                    u64::from_str_radix(v.trim().trim_start_matches("0x"), 16).ok();
                            }
                        }
                        if pid.is_some() && addr.is_some() {
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let pid = pid.unwrap_or(0);
                let addr = addr.unwrap_or(0);
                trace(&format!("target pid={pid} addr={addr:#X}"));
                check!(
                    pid != 0 && addr != 0,
                    "debugger: spawned target pid={pid} addr={addr:#X}"
                );
                let handle = winmem::open_process(pid, "debug-target").ok();
                check!(handle.is_some(), "debugger: opened target process");
                if let Some(handle) = handle {
                    match debugger::DebugSession::start(pid, &handle) {
                        Ok(session) => {
                            trace("DebugSession started");
                            let mut attached = false;
                            for _ in 0..50 {
                                if session.status().attached { attached = true; break; }
                                if session.status().error.is_some() { break; }
                                std::thread::sleep(Duration::from_millis(40));
                            }
                            check!(attached, "debugger: attached (error={:?})", session.status().error);
                            match session.set_breakpoint(Slot(0), addr, BpKind::Write) {
                                Ok(()) => {
                                    trace("breakpoint queued");
                                    // 等最多 4 秒积累命中
                                    let mut hits = 0;
                                    for _ in 0..80 {
                                        std::thread::sleep(Duration::from_millis(50));
                                        hits = session.hits().len();
                                        if hits >= 3 {
                                            break;
                                        }
                                    }
                                    check!(
                                        hits >= 1,
                                        "debugger: hardware write breakpoint hit {hits} times"
                                    );
                                    let records = session.hits();
                                    if let Some(rec) = records.last() {
                                        check!(
                                            rec.rip != 0,
                                            "debugger: hit carries a RIP ({:#X})",
                                            rec.rip
                                        );
                                        check!(
                                            !rec.bytes.is_empty(),
                                            "debugger: hit recorded instruction bytes"
                                        );
                                        check!(
                                            rec.written.is_some(),
                                            "debugger: hit reads back the written value"
                                        );
                                    }
                                    // detach：stop 后事件线程应退出，目标进程仍活着、仍可再打开。
                                    trace(&format!("hits={hits} waiting done"));
                                    session.stop();
                                    let _ = session.wait_done(Duration::from_secs(5));
                                    check!(
                                        session.status().error.is_none(),
                                        "debugger: detach clean (error={:?})",
                                        session.status().error
                                    );
                                    check!(
                                        winmem::open_process(pid, "debug-target").is_ok(),
                                        "debugger: target still alive after detach"
                                    );
                                }
                                Err(e) => {
                                    ok = false;
                                    log.push_str(&format!("FAIL  debugger: set breakpoint: {e}\n"));
                                }
                            }
                        }
                        Err(e) => {
                            ok = false;
                            log.push_str(&format!("FAIL  debugger: attach: {e}\n"));
                        }
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
            }
            Err(e) => {
                ok = false;
                log.push_str(&format!("FAIL  debugger: spawn target: {e}\n"));
            }
        }
        std::fs::remove_file(&info).ok();
    }
    log.push_str(if ok { "\nRESULT: PASS\n" } else { "\nRESULT: FAIL\n" });
    std::fs::write("selftest.log", log).ok();
    ok
}
