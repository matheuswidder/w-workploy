use std::collections::HashMap;

use crate::model::*;

fn unquote_scalar(raw: &str) -> String {
    let t = raw.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        let inner = &t[1..t.len() - 1];
        return inner.replace("\\\"", "\"").replace("\\\\", "\\");
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        let inner = &t[1..t.len() - 1];
        return inner.replace("''", "'");
    }
    if let Some((before, _)) = t.split_once(" #") {
        before.trim().to_string()
    } else {
        t.to_string()
    }
}

fn is_block_scalar_indicator(raw: &str) -> Option<bool> {
    let style = raw.split_once(" #").map(|(a, _)| a.trim()).unwrap_or(raw.trim());
    match style {
        "|" | "|-" | "|+" => Some(false),
        ">" | ">-" | ">+" => Some(true),
        _ => None,
    }
}

pub fn export_yaml(wf: &Workflow) -> String {
    export_yaml_with_extras(wf, &WorkflowExtras::default())
}

/// Serializa a receita incluindo os extras: `x-cpu` / `x-mem` no topo e
/// `x-skip: true` nos passos marcados para pular.
pub fn export_yaml_with_extras(wf: &Workflow, extras: &WorkflowExtras) -> String {
    let on = match wf.trigger {
        Trigger::Push => "push",
        Trigger::Manual | Trigger::Folder => "workflow_dispatch",
    };
    let runner = match wf.runner {
        RunnerKind::Local => "local",
        RunnerKind::Ssh => "self-hosted",
    };
    let mut out = format!("name: {}\non: [{}]\n", wf.name, on);
    if !wf.workdir.trim().is_empty() {
        out.push_str(&format!("workdir: {}\n", wf.workdir.trim()));
    }
    if let Some(c) = extras.cpu_limit {
        out.push_str(&format!("x-cpu: {c}\n"));
    }
    if let Some(m) = extras.mem_limit_mb {
        out.push_str(&format!("x-mem: {m}\n"));
    }
    out.push_str(&format!("\njobs:\n  build:\n    runs-on: {runner}\n    steps:\n"));
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
        if extras.skips(&s.name) {
            out.push_str("        x-skip: true\n");
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

#[allow(clippy::too_many_arguments)]
fn apply_key(
    cur: &mut Option<Step>,
    rest: &str,
    in_block: &mut bool,
    block_indent: &mut usize,
    block_content_indent: &mut Option<usize>,
    block_folded: &mut bool,
    pending_skip: &mut bool,
    raw_line: &str,
) {
    let Some(step) = cur.as_mut() else { return };
    if let Some(v) = rest.strip_prefix("name:") {
        step.name = unquote_scalar(v);
    } else if let Some(v) = rest.strip_prefix("run:") {
        let v = v.trim();
        if let Some(folded) = is_block_scalar_indicator(v) {
            *in_block = true;
            *block_indent = raw_line.len() - raw_line.trim_start().len();
            *block_content_indent = None;
            *block_folded = folded;
        } else {
            step.run = unquote_scalar(v);
        }
    } else if let Some(v) = rest
        .strip_prefix("x-approval:")
        .or_else(|| rest.strip_prefix("approval:"))
    {
        step.approval = unquote_scalar(v).eq_ignore_ascii_case("true");
    } else if let Some(v) = rest
        .strip_prefix("x-skip:")
        .or_else(|| rest.strip_prefix("skip:"))
    {
        *pending_skip = unquote_scalar(v).eq_ignore_ascii_case("true");
    }
}

fn parse_yaml_with_symbolic_id(
    text: &str,
    id: String,
) -> Result<(Workflow, Option<String>, WorkflowExtras), String> {
    let mut name = String::from("Fluxo importado");
    let mut symbolic_id: Option<String> = None;
    let mut trigger = Trigger::Manual;
    let mut runner = RunnerKind::Ssh;
    let mut workdir = String::new();
    let mut on_success = None;
    let mut server_id = None;
    let mut repo_id = None;
    let mut cpu_limit: Option<u32> = None;
    let mut mem_limit_mb: Option<u32> = None;
    let mut skipped_steps: Vec<String> = Vec::new();
    let mut steps: Vec<Step> = Vec::new();
    let mut cur: Option<Step> = None;
    let mut pending_skip = false;

    let mut in_run_block = false;
    let mut block_indent = 0usize;
    let mut block_content_indent: Option<usize> = None;
    let mut block_folded = false;

    let mut in_on_block = false;
    let mut on_indent = 0usize;

    let finish_run_block = |cur: &mut Option<Step>, block_folded: bool| {
        if let Some(s) = cur.as_mut() {
            if block_folded {
                let joined = s
                    .run
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                s.run = joined;
            } else {
                s.run = s.run.trim_end().to_string();
            }
        }
    };

    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();

        if in_run_block {
            if trimmed.is_empty() || indent > block_indent {
                if let Some(s) = cur.as_mut() {
                    if trimmed.is_empty() {
                        s.run.push('\n');
                    } else {
                        let ci = *block_content_indent.get_or_insert(indent);
                        let drop = ci.min(indent);
                        s.run.push_str(&line[drop..]);
                        s.run.push('\n');
                    }
                }
                continue;
            }
            in_run_block = false;
            block_content_indent = None;
            finish_run_block(&mut cur, block_folded);
        }

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if in_on_block {
            if indent > on_indent {
                let low = trimmed.to_ascii_lowercase();
                if low == "push:" || low.starts_with("push:") || low == "- push" {
                    trigger = Trigger::Push;
                }
                continue;
            }
            in_on_block = false;
        }

        if trimmed.starts_with("- ") || trimmed == "-" {
            if let Some(s) = cur.take() {
                if pending_skip && !s.name.trim().is_empty() {
                    skipped_steps.push(s.name.trim().to_string());
                }
                steps.push(s);
            }
            pending_skip = false;
            cur = Some(Step {
                name: String::new(),
                run: String::new(),
                approval: false,
            });
            let rest = trimmed.trim_start_matches('-').trim();
            if !rest.is_empty() {
                apply_key(
                    &mut cur,
                    rest,
                    &mut in_run_block,
                    &mut block_indent,
                    &mut block_content_indent,
                    &mut block_folded,
                    &mut pending_skip,
                    line,
                );
            }
            continue;
        }

        if cur.is_none()
            && steps.is_empty()
            && let Some(v) = trimmed.strip_prefix("name:")
        {
            let val = unquote_scalar(v);
            if !val.is_empty() {
                name = val;
            }
            continue;
        }
        if cur.is_none()
            && steps.is_empty()
            && let Some(v) = trimmed.strip_prefix("id:")
        {
            let val = unquote_scalar(v);
            if !val.is_empty() {
                symbolic_id = Some(val);
            }
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("on:") {
            let val = unquote_scalar(v);
            if val.is_empty() {
                in_on_block = true;
                on_indent = indent;
            } else if val.to_ascii_lowercase().contains("push") {
                trigger = Trigger::Push;
            }
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("trigger:") {
            let val = unquote_scalar(v).to_ascii_lowercase();
            trigger = match val.as_str() {
                "push" => Trigger::Push,
                "folder" | "watch" => Trigger::Folder,
                _ => Trigger::Manual,
            };
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("workdir:") {
            let val = unquote_scalar(v);
            if !val.is_empty() {
                workdir = val;
            }
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("runs-on:") {
            let val = unquote_scalar(v);
            runner = if val.eq_ignore_ascii_case("local") {
                RunnerKind::Local
            } else {
                RunnerKind::Ssh
            };
            continue;
        }
        if let Some(v) = trimmed
            .strip_prefix("x-chain:")
            .or_else(|| trimmed.strip_prefix("on_success:"))
        {
            let val = unquote_scalar(v);
            if !val.is_empty() {
                on_success = Some(val);
            }
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("x-server:") {
            let val = unquote_scalar(v);
            if !val.is_empty() {
                server_id = Some(val);
            }
            continue;
        }
        if let Some(v) = trimmed.strip_prefix("x-repo:") {
            let val = unquote_scalar(v);
            if !val.is_empty() {
                repo_id = Some(val);
            }
            continue;
        }
        if let Some(v) = trimmed
            .strip_prefix("x-cpu:")
            .or_else(|| trimmed.strip_prefix("cpu_limit:"))
        {
            if let Ok(n) = unquote_scalar(v).parse::<u32>() {
                cpu_limit = (n > 0).then_some(n);
            }
            continue;
        }
        if let Some(v) = trimmed
            .strip_prefix("x-mem:")
            .or_else(|| trimmed.strip_prefix("x-memory:")
                .or_else(|| trimmed.strip_prefix("mem_limit_mb:")))
        {
            if let Ok(n) = unquote_scalar(v).parse::<u32>() {
                mem_limit_mb = (n > 0).then_some(n);
            }
            continue;
        }
        if cur.is_some() {
            apply_key(
                &mut cur,
                trimmed,
                &mut in_run_block,
                &mut block_indent,
                &mut block_content_indent,
                &mut block_folded,
                &mut pending_skip,
                line,
            );
        }
    }

    if in_run_block {
        finish_run_block(&mut cur, block_folded);
    }
    if let Some(s) = cur.take() {
        if pending_skip && !s.name.trim().is_empty() {
            skipped_steps.push(s.name.trim().to_string());
        }
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
    Ok((
        Workflow {
            id,
            name,
            trigger,
            runner,
            workdir,
            steps,
            on_success,
            server_id,
            repo_id,
            source: None,
        },
        symbolic_id,
        WorkflowExtras {
            cpu_limit,
            mem_limit_mb,
            skipped_steps,
        },
    ))
}

pub fn import_yaml(text: &str, id: String) -> Result<Workflow, String> {
    parse_yaml_with_symbolic_id(text, id).map(|(wf, _, _)| wf)
}

/// Importa uma receita e devolve também os extras (limites de CPU/RAM e
/// passos marcados como "pular").
pub fn import_yaml_with_extras(
    text: &str,
    id: String,
) -> Result<(Workflow, WorkflowExtras), String> {
    parse_yaml_with_symbolic_id(text, id).map(|(wf, _, extras)| (wf, extras))
}

/// Importa um YAML que pode ser uma receita única (formato clássico) ou um
/// documento com a chave `workflows:` listando várias receitas. Cada receita
/// da lista é reutilizada por `import_yaml`, recebendo um id novo, e referências
/// simbólicas em `on_success:` / `x-chain:` (`id:` ou nome de outra receita do
/// mesmo arquivo) são resolvidas para o id real gerado.
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
    let mut ref_to_id: HashMap<String, String> = HashMap::new();
    let mut extras_by_id: HashMap<String, WorkflowExtras> = HashMap::new();

    for (idx, item) in items.iter().enumerate() {
        let (wf, sym_id, extras) = parse_yaml_with_symbolic_id(item, new_id("wf")).map_err(|e| {
            format!("Receita {} da lista 'workflows' é inválida: {e}", idx + 1)
        })?;
        if let Some(sym) = sym_id {
            ref_to_id.insert(sym, wf.id.clone());
        }
        ref_to_id.insert(wf.name.to_ascii_lowercase(), wf.id.clone());
        if !extras.is_empty() {
            extras_by_id.insert(wf.id.clone(), extras);
        }
        out.push(wf);
    }

    for wf in &mut out {
        if let Some(target) = &wf.on_success
            && let Some(resolved) = ref_to_id
                .get(target)
                .or_else(|| ref_to_id.get(&target.to_ascii_lowercase()))
        {
            wf.on_success = Some(resolved.clone());
        }
    }

    Ok(out)
}

/// Receitas importadas com seus extras (limites e passos pulados), indexados
/// pelo id real de cada receita.
#[derive(Debug, Default)]
pub struct ImportedRecipes {
    pub workflows: Vec<Workflow>,
    pub extras: HashMap<String, WorkflowExtras>,
}

/// Igual a [`import_yaml_multi`], mas devolve também os extras de cada receita.
pub fn import_yaml_multi_extras(text: &str) -> Result<ImportedRecipes, String> {
    let is_list_doc = text.lines().any(|l| {
        let trimmed = l.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return false;
        }
        let has_indent = l.len() > l.trim_start().len();
        !has_indent && trimmed.starts_with("workflows:")
    });
    if !is_list_doc {
        let (wf, extras) = import_yaml_with_extras(text, new_id("wf"))?;
        let mut map = HashMap::new();
        if !extras.is_empty() {
            map.insert(wf.id.clone(), extras);
        }
        return Ok(ImportedRecipes {
            workflows: vec![wf],
            extras: map,
        });
    }
    let items = split_workflow_list_items(text);
    if items.is_empty() {
        return Err("Nenhuma receita encontrada na lista 'workflows'.".into());
    }
    let mut out = Vec::with_capacity(items.len());
    let mut ref_to_id: HashMap<String, String> = HashMap::new();
    let mut extras_by_id: HashMap<String, WorkflowExtras> = HashMap::new();

    for (idx, item) in items.iter().enumerate() {
        let (wf, sym_id, extras) = parse_yaml_with_symbolic_id(item, new_id("wf")).map_err(|e| {
            format!("Receita {} da lista 'workflows' é inválida: {e}", idx + 1)
        })?;
        if let Some(sym) = sym_id {
            ref_to_id.insert(sym, wf.id.clone());
        }
        ref_to_id.insert(wf.name.to_ascii_lowercase(), wf.id.clone());
        if !extras.is_empty() {
            extras_by_id.insert(wf.id.clone(), extras);
        }
        out.push(wf);
    }

    for wf in &mut out {
        if let Some(target) = &wf.on_success
            && let Some(resolved) = ref_to_id
                .get(target)
                .or_else(|| ref_to_id.get(&target.to_ascii_lowercase()))
        {
            wf.on_success = Some(resolved.clone());
        }
    }

    Ok(ImportedRecipes {
        workflows: out,
        extras: extras_by_id,
    })
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
                workdir: "/srv/app".into(),
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
            assert_eq!(imported.workdir, "/srv/app");
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
        assert_eq!(list[0].steps[1].run.trim_end(), "pnpm test\npnpm lint");
        assert_ne!(list[0].id, list[1].id, "cada receita recebe id próprio");
        assert_eq!(list[1].name, "Deploy VPS");
        assert_eq!(list[1].runner, RunnerKind::Ssh);
        assert_eq!(list[1].server_id.as_deref(), Some("vps-prod"));
        assert!(list[1].steps[0].approval);
    }

    #[test]
    fn import_yaml_multi_resolves_symbolic_id_and_workploy_shorthand_fields() {
        let doc = "\
workflows:
  - id: wf-ci
    name: CI Principal
    trigger: push
    on_success: wf-deploy
    steps:
      - name: Testes
        run: |-
          if [ -f package.json ]; then
            pnpm test
          fi
  - id: wf-deploy
    name: Deploy Produção
    trigger: manual
    workdir: /srv/app
    steps:
      - name: Rebuild
        run: docker compose up -d --build
        approval: true
";
        let list = import_yaml_multi(doc).expect("formato shorthand deve importar");
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].trigger, Trigger::Push);
        assert_eq!(list[0].on_success.as_deref(), Some(list[1].id.as_str()));
        assert!(list[0].steps[0].run.contains("  pnpm test"));
        assert_eq!(list[1].workdir, "/srv/app");
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

    #[test]
    fn acme_central_recipe_imports_with_parser_real() {
        // Fixture inline: nenhum repositório irmão é lido, então o teste não
        // depende de um checkout específico da máquina do desenvolvedor.
        let text = r#"
workflows:
  - id: ci
    name: CI Acme
    on: [push]
    jobs:
      build:
        runs-on: self-hosted
        steps:
          - name: Instalar
            run: echo install
          - name: Tipos
            run: echo types
          - name: Testes
            run: echo test
          - name: Smoke
            run: echo smoke
  - id: deploy
    name: Deploy Acme VPS
    on: [workflow_dispatch]
    jobs:
      deploy:
        runs-on: self-hosted
        steps:
          - name: Pull
            run: echo pull
          - name: Up
            run: echo up
          - name: Health
            run: echo health
"#;
        let list = import_yaml_multi(text).expect("receita do Acme deve importar");
        assert_eq!(list.len(), 2, "CI e Deploy");
        let ci = list.iter().find(|w| w.name == "CI Acme").unwrap();
        assert_eq!(ci.trigger, Trigger::Push);
        assert_eq!(ci.runner, RunnerKind::Ssh);
        assert_eq!(ci.steps.len(), 4);
        let deploy = list
            .iter()
            .find(|w| w.name == "Deploy Acme VPS")
            .expect("deploy presente");
        assert_eq!(deploy.trigger, Trigger::Manual);
        assert_eq!(deploy.steps.len(), 3);
    }

    #[test]
    fn acme_mirror_recipes_import_with_parser_real() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("w-erp-central")
            .join("infra")
            .join("workploy");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return; // repositório irmão ausente — nada a validar
        };
        let mut checked = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            let is_yml = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "yml" || e == "yaml");
            if !is_yml {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let wf = import_yaml(&text, new_id("wf"))
                .unwrap_or_else(|e| panic!("{} deve importar: {e}", path.display()));
            assert!(!wf.steps.is_empty(), "{} não pode ficar sem passos", path.display());
            checked += 1;
        }
        assert!(checked > 0, "nenhum espelho de receita encontrado");
    }
}

#[cfg(test)]
mod extras_tests {
    use super::*;

    const DOC: &str = "\
name: CI com limites
on: [push]
x-cpu: 2
x-mem: 4096
jobs:
  build:
    runs-on: self-hosted
    steps:
      - name: Instalar
        run: pnpm install
      - name: Testes pesados
        run: pnpm test
        x-skip: true
      - name: Deploy
        run: docker compose up -d
";

    #[test]
    fn importa_limites_e_passos_pulados() {
        let (wf, extras) = import_yaml_with_extras(DOC, "wf-x".into()).unwrap();
        assert_eq!(extras.cpu_limit, Some(2));
        assert_eq!(extras.mem_limit_mb, Some(4096));
        assert_eq!(extras.skipped_steps, vec!["Testes pesados".to_string()]);
        assert!(extras.skips("Testes pesados"));
        assert!(!extras.skips("Instalar"));
        assert_eq!(wf.steps.len(), 3);
    }

    #[test]
    fn exporta_limites_e_passos_pulados() {
        let (wf, extras) = import_yaml_with_extras(DOC, "wf-x".into()).unwrap();
        let text = export_yaml_with_extras(&wf, &extras);
        assert!(text.contains("x-cpu: 2"), "{text}");
        assert!(text.contains("x-mem: 4096"), "{text}");
        assert!(text.contains("x-skip: true"), "{text}");
        // Ida e volta preserva os limites.
        let (_, back) = import_yaml_with_extras(&text, "wf-y".into()).unwrap();
        assert_eq!(back.cpu_limit, Some(2));
        assert_eq!(back.mem_limit_mb, Some(4096));
        assert_eq!(back.skipped_steps.len(), 1);
    }
}
