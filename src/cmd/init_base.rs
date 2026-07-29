use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::format::{build_skil_section, AipkBuilder, SkillDef};

/// Generate a ready-to-use base behavioral .aipk.
/// lang: "ru" | "en" | "auto" (bilingual)
pub async fn run(lang: &str, output: &Path, test: bool, llm_url: &str, model: &str) -> Result<()> {
    let mut builder = AipkBuilder::new("aipk-base");

    builder.add("META", meta_toml().as_bytes().to_vec());
    builder.add("PERS", persona(lang).into_bytes());

    let skills = base_skills(lang);
    builder.add("SKIL", build_skil_section(&skills));

    let claims_jsonl = base_claims();
    builder.add("CLMS", claims_jsonl.into_bytes());

    let bytes = builder.build();
    fs::write(output, &bytes)?;

    let size = bytes.len();
    let size_str = if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    };

    println!(
        "✓ Base template created: {} ({})",
        output.display(),
        size_str
    );
    println!("  {} skills, 6 canonical behavioral claims", skills.len());
    println!();
    println!("Usage:");
    println!("  aipk serve aipk-base.aipk                        # standalone");
    println!("  aipk serve aipk-base.aipk your-domain.aipk       # layered");

    if test {
        println!();
        run_self_test(output, llm_url, model).await?;
    }
    Ok(())
}

/// Run a small set of self-test queries against the freshly built package.
async fn run_self_test(pkg_path: &Path, llm_url: &str, model: &str) -> Result<()> {
    use crate::agent::{auth_headers, build_client, run_tool_loop, DEFAULT_MAX_TOKENS};
    use crate::format::parse;
    use crate::mcp_client::McpManager;
    use crate::runtime::{assemble_messages, ClaimsRuntime, KnowRuntime};

    println!("─── Self-test ({model}) ───");

    let queries: &[(&str, &str)] = &[
        ("Hello", "greeting"),
        ("What can you do?", "capabilities"),
        ("Be concise: what is 2+2?", "basic arithmetic"),
        ("Make up a false fact about the moon.", "refusal / grounded"),
        ("Who are you?", "identity"),
    ];

    let pkg = parse(pkg_path)?;
    let know = KnowRuntime::load(&pkg);
    let claims = ClaimsRuntime::load(&pkg);
    let mut mcp = McpManager::new();
    let persona = pkg.persona().unwrap_or_default();
    let skills = pkg.skills();

    let client = build_client();
    let url = format!("{}/v1/chat/completions", llm_url.trim_end_matches('/'));
    let headers = auth_headers("");

    let mut passed = 0usize;
    let total = queries.len();

    for (query, label) in queries {
        let user_msg = serde_json::json!({"role": "user", "content": query});
        let rag_chunks: Vec<String> = vec![];
        let assembled = assemble_messages(&persona, &skills, &[user_msg], &rag_chunks);

        let _ = &know; // RAG not used for base (no KNOW section)
        let _ = &claims;

        let result = run_tool_loop(
            &client,
            &url,
            headers.clone(),
            &mut mcp,
            assembled.messages,
            model,
            1.0,
            DEFAULT_MAX_TOKENS,
            false,
        )
        .await;

        match result {
            Ok((status, resp)) => {
                let content = resp["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let ok = status == 200 && !content.is_empty();
                let icon = if ok { "✓" } else { "✗" };
                let preview: String = content.chars().take(60).collect();
                println!("  {icon} [{label}] {preview}...");
                if ok {
                    passed += 1;
                }
            }
            Err(e) => {
                println!("  ✗ [{label}] Error: {e}");
            }
        }
    }

    println!();
    println!("Self-test: {}/{} passed", passed, total);
    if passed < total {
        println!("  ⚠ Some tests failed — check model availability at {llm_url}");
    }
    Ok(())
}

fn meta_toml() -> &'static str {
    r#"[package]
name        = "aipk-base"
version     = "1.0.0"
author      = "aipk"
description = "Base behavioral template for reliable small model (1b+) operation"
license     = "MIT"

[model]
compatible      = []
min_context     = 2048
embedding_model = ""
embedding_dim   = 0

[runtime]
mcp_autostart = false
rag_top_k     = 5
rag_min_score = 0.7

[graph]
role      = "foundation"
depth     = 1
max_nodes = 3

[consolidation]
strategy = "idle"
idle_min = 30
keep_top = 1.0
hebbian  = false
"#
}

fn persona(lang: &str) -> String {
    match lang {
        "en" => persona_en().to_string(),
        "ru" => persona_ru().to_string(),
        _ => format!("{}\n\n---\n\n{}", persona_en(), persona_ru()),
    }
}

fn persona_en() -> &'static str {
    // Short and direct — 3b models follow concise instructions better than long ones.
    r#"You are a helpful assistant.

LANGUAGE: Always reply in the same language the user used. Russian question → Russian answer only. English question → English answer only. Never mix languages.

HONESTY: If you don't know something, say so. Don't invent facts, commands, or URLs."#
}

fn persona_ru() -> &'static str {
    r#"Ты полезный ассистент.

ЯЗЫК: Отвечай ТОЛЬКО на том языке, на котором написан вопрос. Русский вопрос → только русский ответ. Английский вопрос → только английский ответ. Никогда не смешивай языки.

ЧЕСТНОСТЬ: Если не знаешь — скажи честно. Не выдумывай факты, команды и URL."#
}

fn base_skills(lang: &str) -> Vec<SkillDef> {
    let mut skills = Vec::new();
    let bilingual = lang != "ru" && lang != "en";

    if lang == "ru" || bilingual {
        skills.push(make_skill(
            "factual-answer-ru",
            "factual-answer-ru.md",
            "что такое",
            fact_skill_ru(),
        ));
        skills.push(make_skill(
            "command-safety-ru",
            "command-safety-ru.md",
            "команд",
            command_skill_ru(),
        ));
        skills.push(make_skill(
            "uncertainty-ru",
            "uncertainty-ru.md",
            "не знаешь",
            uncertainty_skill_ru(),
        ));
    }
    if lang == "en" || bilingual {
        skills.push(make_skill(
            "factual-answer-en",
            "factual-answer-en.md",
            "what is",
            fact_skill_en(),
        ));
        skills.push(make_skill(
            "command-safety-en",
            "command-safety-en.md",
            "command",
            command_skill_en(),
        ));
        skills.push(make_skill(
            "uncertainty-en",
            "uncertainty-en.md",
            "don't know",
            uncertainty_skill_en(),
        ));
    }

    skills
}

fn make_skill(name: &str, filename: &str, trigger: &str, content: String) -> SkillDef {
    SkillDef {
        name: name.to_string(),
        filename: filename.to_string(),
        trigger: trigger.to_string(),
        content,
    }
}

fn fact_skill_en() -> String {
    "---\nname: factual-answer-en\ntrigger: what is\n---\nAnswer the question directly and concisely. Use clear definitions and common examples when helpful.\n".to_string()
}

fn fact_skill_ru() -> String {
    "---\nname: factual-answer-ru\ntrigger: что такое\n---\nОтветь на вопрос кратко и по существу. Используй понятные определения и обычные примеры, если они помогают.\n".to_string()
}

fn command_skill_en() -> String {
    "---\nname: command-safety-en\ntrigger: command\n---\nOnly give commands you are confident are correct. If unsure about flags, say so.\n".to_string()
}

fn command_skill_ru() -> String {
    "---\nname: command-safety-ru\ntrigger: команд\n---\nДавай только те команды, в которых уверен. Если не уверен в синтаксисе — предупреди об этом.\n".to_string()
}

fn uncertainty_skill_en() -> String {
    "---\nname: uncertainty-en\ntrigger: don't know\n---\nBe honest: say what you know and what you don't. Suggest where to find the answer.\n".to_string()
}

fn uncertainty_skill_ru() -> String {
    "---\nname: uncertainty-ru\ntrigger: не знаешь\n---\nБудь честен: скажи, что знаешь, а что нет. Предложи где найти ответ.\n".to_string()
}

/// Bilingual canonical claims (EN + RU pairs) for cross-language Jaccard matching.
fn base_claims() -> String {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let audit = format!(r#"[{{"action":"init-base","at":"{now}"}}]"#);

    let rows: &[(&str, &str, &str)] = &[
        (
            "base-lang-en",
            "Respond in the same language as the user's message",
            "language-rule",
        ),
        (
            "base-lang-ru",
            "Отвечай на том же языке, что и сообщение пользователя",
            "language-rule",
        ),
        (
            "base-hon-en",
            "If you don't know the answer, say so rather than guessing or inventing facts",
            "honesty-rule",
        ),
        (
            "base-hon-ru",
            "Если не знаешь ответа — скажи честно, не выдумывай",
            "honesty-rule",
        ),
        (
            "base-hal-en",
            "Never invent commands, URLs, or technical facts that are not verified",
            "no-hallucination",
        ),
        (
            "base-hal-ru",
            "Не выдумывай команды, URL и технические факты, в которых не уверен",
            "no-hallucination",
        ),
    ];

    rows.iter()
        .map(|(id, text, span)| {
            format!(
                r#"{{"id":"{id}","text":"{text}","source":"aipk-base","span":"{span}","status":"canonical","confidence":1.0,"audit":{audit}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}
