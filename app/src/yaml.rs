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
        source: None,
    })
}

/// Importa um YAML que pode ser uma receita única (formato clássico) ou um
/// documento com a chave `workflows:` listando várias receitas. Cada receita
/// da lista é reutilizada por `import_yaml`, recebendo um id novo.
pub fn import_yaml_multi(text: &str) -> Result<Vec<Workflow>, String> {
    let is_list_doc = text.lines().any(|l| {
        let trimmed = l.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return false;
        }
        let has_indent = l.len() > l.trim_start().len();
        !has_indent && trimmed.starts_with("workflows:")
    });
    if !is_list_doc {
        let wf = import_yaml(text, new_id("wf"))?;
        return Ok(vec![wf]);
    }
    let items = split_workflow_list_items(text);
    if items.is_empty() {
        return Err("Nenhuma receita encontrada na lista 'workflows'.".into());
    }
    let mut out = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let wf = import_yaml(item, new_id("wf")).map_err(|e| {
            format!("Receita {} da lista 'workflows' é inválida: {e}", idx + 1)
        })?;
        out.push(wf);
    }
    Ok(out)
}

/// Coluna da primeira chave de um item de lista (`"  - name:"` → 4).
fn list_item_key_indent(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b' ') {
        i += 1;
    }
    i
}

/// Divide um documento com `workflows:` no corpo de cada receita da lista,
/// dedentado para o formato de receita única que `import_yaml` entende.
/// Linhas antes do primeiro item (a própria chave `workflows:`, comentários)
/// são ignoradas; um novo item começa em `- ` com o mesmo recuo do primeiro.
fn split_workflow_list_items(text: &str) -> Vec<String> {
    let mut items: Vec<String> = Vec::new();
    // (recuo da chave, buffer do item em andamento)
    let mut cur: Option<(usize, String)> = None;
    let mut item_indent: Option<usize> = None;
    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            if let Some((_, buf)) = cur.as_mut() {
                buf.push_str(line);
                buf.push('\n');
            }
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let starts_item = trimmed == "-" || trimmed.starts_with("- ");
        if starts_item && item_indent.is_none_or(|base| indent == base) {
            if let Some((key_indent, buf)) = cur.take()
                && !buf.trim().is_empty()
            {
                items.push(dedent_block(&buf, key_indent));
            }
            let key_indent = list_item_key_indent(line);
            item_indent = Some(indent);
            // Troca o marcador `-` por espaço: a primeira chave fica no mesmo
            // recuo das seguintes e o bloco dedenta por igual.
            let mut first = String::with_capacity(line.len());
            first.push_str(&line[..indent]);
            first.push(' ');
            first.push_str(&line[indent + 1..]);
            cur = Some((key_indent, format!("{first}\n")));
        } else if let Some((_, buf)) = cur.as_mut() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    if let Some((key_indent, buf)) = cur.take()
        && !buf.trim().is_empty()
    {
        items.push(dedent_block(&buf, key_indent));
    }
    items
}

/// Remove até `key_indent` espaços do início de cada linha do bloco.
fn dedent_block(buf: &str, key_indent: usize) -> String {
    let mut out = String::with_capacity(buf.len());
    for raw in buf.lines() {
        let bytes = raw.as_bytes();
        let mut drop = 0;
        while drop < key_indent && drop < bytes.len() && bytes[drop] == b' ' {
            drop += 1;
        }
        out.push_str(&raw[drop..]);
        out.push('\n');
    }
    out
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
                source: None,
            };
            let imported = import_yaml(&export_yaml(&wf), "a".into()).unwrap();
            assert_eq!(imported.runner, runner);
            assert_eq!(imported.on_success, wf.on_success);
            assert_eq!(imported.server_id, wf.server_id);
            assert_eq!(imported.repo_id, wf.repo_id);
        }
    }

    #[test]
    fn repo_root_workploy_yml_imports_cleanly() {
        let raw = include_str!("../../.workploy.yml");
        let wf = import_yaml(raw, "wf-self".into()).expect(".workploy.yml deve importar");
        assert_eq!(wf.name, "Workploy Windows CI & Release");
        assert_eq!(wf.trigger, Trigger::Push);
        assert_eq!(wf.runner, RunnerKind::Local);
        assert_eq!(wf.steps.len(), 4);
        assert!(!wf.steps[0].approval);
        assert!(wf.steps[3].approval);
    }

    #[test]
    fn import_yaml_multi_aceita_formato_antigo_de_receita_unica() {
        let raw = include_str!("../../.workploy.yml");
        let list = import_yaml_multi(raw).expect("receita única deve continuar importando");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Workploy Windows CI & Release");
        assert_eq!(list[0].steps.len(), 4);
    }

    #[test]
    fn import_yaml_multi_importa_lista_de_receitas() {
        let doc = "\
workflows:
  - name: CI do Site
    on: [push]
    jobs:
      build:
        runs-on: local
        steps:
          - name: Instalar dependências
            run: pnpm install --frozen-lockfile
          - name: Testes
            run: |
              pnpm test
              pnpm lint
  - name: Deploy VPS
    on: [workflow_dispatch]
    x-server: vps-prod
    jobs:
      build:
        runs-on: self-hosted
        steps:
          - name: Subir serviços
            run: docker compose up -d
            x-approval: true
";
        let list = import_yaml_multi(doc).expect("lista deve importar");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "CI do Site");
        assert_eq!(list[0].trigger, Trigger::Push);
        assert_eq!(list[0].runner, RunnerKind::Local);
        assert_eq!(list[0].steps.len(), 2);
        assert_eq!(list[0].steps[0].name, "Instalar dependências");
        // O parser clássico de bloco `run: |` mantém a quebra final quando o
        // documento acaba dentro do bloco; a lista preserva essa paridade.
        assert_eq!(list[0].steps[1].run.trim_end(), "pnpm test\npnpm lint");
        assert_ne!(list[0].id, list[1].id, "cada receita recebe id próprio");
        assert_eq!(list[1].name, "Deploy VPS");
        assert_eq!(list[1].runner, RunnerKind::Ssh);
        assert_eq!(list[1].server_id.as_deref(), Some("vps-prod"));
        assert!(list[1].steps[0].approval);
    }

    #[test]
    fn import_yaml_multi_erro_aponta_a_receita_invalida() {
        let doc = "workflows:\n  - name: Boa\n    jobs:\n      build:\n        steps:\n          - name: ok\n            run: echo ok\n  - name: Vazia\n    on: [push]\n";
        let err = import_yaml_multi(doc).unwrap_err();
        assert!(err.contains("Receita 2"), "erro deve citar a receita 2: {err}");
        assert!(err.contains("steps"), "erro deve citar o motivo: {err}");
    }

    #[test]
    fn import_yaml_multi_lista_vazia_da_erro_claro() {
        let err = import_yaml_multi("workflows: []\n").unwrap_err();
        assert!(err.contains("'workflows'"), "erro deve citar a lista: {err}");
    }
}
