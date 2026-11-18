use crate::model::*;

pub fn export_yaml(wf: &Workflow) -> String {
    let on = match wf.trigger {
        Trigger::Push => "push",
        Trigger::Manual | Trigger::Folder => "workflow_dispatch",
    };
    let runner = match wf.runner {
        RunnerKind::Local => "local",
        RunnerKind::Ssh => "self-hosted",
    };
    let mut out = format!(
        "name: {}\non: [{}]\n\njobs:\n  build:\n    runs-on: {runner}\n    steps:\n",
        wf.name, on
    );
    for s in &wf.steps {
        out.push_str(&format!("      - name: {}\n", s.name));
        if s.run.contains('\n') {
            out.push_str("        run: |\n");
            for line in s.run.lines() {
                out.push_str(&format!("          {line}\n"));
            }
        } else {
            out.push_str(&format!("        run: {}\n", s.run));
        }
        if s.approval {
            out.push_str("        x-approval: true\n");
        }
    }
    if let Some(next) = &wf.on_success {
        out.push_str(&format!("x-chain: {next}\n"));
    }
    if let Some(srv) = &wf.server_id {
        out.push_str(&format!("x-server: {srv}\n"));
    }
    if let Some(repo) = &wf.repo_id {
        out.push_str(&format!("x-repo: {repo}\n"));
    }
    out
}

fn apply_key(
    cur: &mut Option<Step>,
    rest: &str,
    in_block: &mut bool,
    block_indent: &mut usize,
    raw_line: &str,
) {
    let Some(step) = cur.as_mut() else { return };
    if let Some(v) = rest.strip_prefix("name:") {
        step.name = v.trim().trim_matches('"').to_string();
    } else if let Some(v) = rest.strip_prefix("run:") {
        let v = v.trim();
        if v == "|" || v == ">" {
            *in_block = true;
            *block_indent = raw_line.len() - raw_line.trim_start().len();
        } else {
            step.run = v.trim_matches('"').to_string();
        }
    } else if let Some(v) = rest.strip_prefix("x-approval:") {
        step.approval = v.trim() == "true";
    }
}

pub fn import_yaml(text: &str, id: String) -> Result<Workflow, String> {
    let mut name = String::from("Fluxo importado");
    let mut trigger = Trigger::Manual;
    let mut runner = RunnerKind::Ssh;
    let mut on_success = None;
    let mut server_id = None;
    let mut repo_id = None;
    let mut steps: Vec<Step> = Vec::new();
    let mut cur: Option<Step> = None;
    let mut in_run_block = false;
    let mut block_indent = 0usize;

    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if in_run_block {
            let indent = line.len() - line.trim_start().len();
            if trimmed.is_empty() || indent > block_indent {
                if let Some(s) = cur.as_mut() {
                    s.run.push_str(trimmed);
                    s.run.push('\n');
                }
                continue;
            }
            in_run_block = false;
            if let Some(s) = cur.as_mut() {
                s.run = s.run.trim_end().to_string();
            }
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("- ") || trimmed == "-" {
            if let Some(s) = cur.take() {
                steps.push(s);
            }
            cur = Some(Step {
                name: String::new(),
                run: String::new(),
                approval: false,
            });
            let rest = trimmed.trim_start_matches('-').trim();
            if !rest.is_empty() {
                apply_key(&mut cur, rest, &mut in_run_block, &mut block_indent, line);
            }
            continue;
        }
        if cur.is_none()
            && steps.is_empty()
            && let Some(v) = trimmed.strip_prefix("name:")
        {
            name = v.trim().trim_matches('"').to_string();
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("on:") {
            if v.contains("push") {
                trigger = Trigger::Push;
            }
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("runs-on:") {
            runner = if v.trim().trim_matches('"') == "local" {
                RunnerKind::Local
            } else {
                RunnerKind::Ssh
            };
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("x-chain:") {
            on_success = Some(v.trim().trim_matches('"').to_string());
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("x-server:") {
            let val = v.trim().trim_matches('"');
            if !val.is_empty() {
                server_id = Some(val.to_string());
            }
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("x-repo:") {
            let val = v.trim().trim_matches('"');
            if !val.is_empty() {
                repo_id = Some(val.to_string());
            }
            continue;
        }
        if cur.is_some() {
            apply_key(&mut cur, trimmed, &mut in_run_block, &mut block_indent, line);
        }
    }
    if let Some(s) = cur.take() {
        steps.push(s);
    }
    if steps.is_empty() {
        return Err("Nenhum passo (steps) encontrado no YAML.".into());
    }
    for (i, s) in steps.iter_mut().enumerate() {
        if s.name.is_empty() {
            s.name = format!("Passo {}", i + 1);
        }
    }
    Ok(Workflow {
        id,
        name,
        trigger,
        runner,
        workdir: String::new(),
        steps,
        on_success,
        server_id,
        repo_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_runner_chain_and_routing() {
        for runner in [RunnerKind::Local, RunnerKind::Ssh] {
            let wf = Workflow {
                id: "a".into(),
                name: "teste".into(),
                trigger: Trigger::Push,
                runner,
                workdir: String::new(),
                steps: vec![Step {
                    name: "teste".into(),
                    run: "echo ok".into(),
                    approval: false,
                }],
                on_success: Some("next".into()),
                server_id: Some("vps-1".into()),
                repo_id: Some("rb-1".into()),
            };
            let imported = import_yaml(&export_yaml(&wf), "a".into()).unwrap();
            assert_eq!(imported.runner, runner);
            assert_eq!(imported.on_success, wf.on_success);
            assert_eq!(imported.server_id, wf.server_id);
            assert_eq!(imported.repo_id, wf.repo_id);
        }
    }
}
