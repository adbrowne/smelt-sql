//! Phase 2c — LSP goto-definition tests for `smelt.<path>` path refs.
//!
//! Test 5: `goto_path_ref_jumps_to_owning_file`
//!   - Sets up a workspace with a `models/users.sql` and a `models/downstream.sql`
//!     that references `smelt.models.users`.
//!   - Verifies that `resolve_ref_path` returns the right file.
//!   - Verifies that `symbol_at_cursor` returns `PathRef { segments: ["models", "users"] }`
//!     when the cursor is on the `smelt.models.users` token.

use std::path::PathBuf;

use smelt_db::{resolve_ref_path, Database, RefKind, SourceFile, Workspace};
use smelt_parser::{
    ast::File as AstFile,
    symbol::{symbol_at_cursor, SymbolAtCursor},
};

// ---------------------------------------------------------------------------
// Workspace helper
// ---------------------------------------------------------------------------

struct TestWs {
    _temp: tempfile::TempDir,
    db: Database,
    project_root: PathBuf,
    files: Vec<PathBuf>,
}

impl TestWs {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().to_path_buf();
        let mut db = Database::default();
        db.set_project_input(project_root.clone(), String::new());
        db.set_workspace(Vec::new(), Vec::new());
        Self {
            _temp: temp,
            db,
            project_root,
            files: Vec::new(),
        }
    }

    fn add_sql(&mut self, rel_path: &str, sql: &str) -> PathBuf {
        let path = self.project_root.join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, sql).unwrap();
        self.db
            .set_source_file(path.clone(), sql.to_string(), self.project_root.clone());
        self.files.push(path.clone());
        self.sync();
        path
    }

    fn sync(&mut self) {
        let files: Vec<SourceFile> = self
            .files
            .iter()
            .filter_map(|p| self.db.source_file(p))
            .collect();
        let projects = self
            .db
            .project_input(&self.project_root)
            .into_iter()
            .collect::<Vec<_>>();
        self.db.set_workspace(files, projects);
    }

    fn workspace(&self) -> Workspace {
        Workspace::try_get(&self.db).expect("workspace set")
    }
}

// ---------------------------------------------------------------------------
// Test 5: goto_path_ref_jumps_to_owning_file
// ---------------------------------------------------------------------------

#[test]
fn goto_path_ref_jumps_to_owning_file() {
    let mut ws = TestWs::new();
    let users_path = ws.add_sql(
        "models/users.sql",
        "SELECT 1 AS user_id, 'alice' AS user_name\n",
    );
    let downstream_sql = "SELECT * FROM smelt.models.users\n";
    ws.add_sql("models/downstream.sql", downstream_sql);

    // 1. Path resolution via smelt-db.
    let resolved = resolve_ref_path(
        &ws.db,
        ws.workspace(),
        vec!["models".to_string(), "users".to_string()],
    )
    .expect("resolve_ref_path must find models/users.sql");

    assert_eq!(
        resolved.kind,
        RefKind::Model,
        "users.sql must resolve as Model kind"
    );
    let src_file = resolved
        .source_file
        .expect("Model kind must have a source_file");
    assert_eq!(
        src_file.path(&ws.db),
        &users_path,
        "source_file must point to users.sql"
    );

    // 2. symbol_at_cursor on the `smelt.models.users` text.
    let parse = smelt_parser::parse(downstream_sql);
    let syntax = parse.syntax();
    let ast = AstFile::cast(syntax).expect("parses as AstFile");

    // The cursor is placed on 'users' (last segment).
    // smelt.models.users starts at offset 14 (after "SELECT * FROM ").
    let users_token_offset = downstream_sql.find("smelt.models.users").unwrap() + 14; // well inside
    let sym = symbol_at_cursor(&ast, downstream_sql, users_token_offset);

    match sym {
        Some(SymbolAtCursor::PathRef { segments }) => {
            assert_eq!(
                segments,
                vec!["models".to_string(), "users".to_string()],
                "PathRef segments must be [models, users]; got {segments:?}"
            );
        }
        other => panic!(
            "expected SymbolAtCursor::PathRef for smelt.models.users at offset {users_token_offset}, got {other:?}"
        ),
    }
}

// ---------------------------------------------------------------------------
// Test 5b: cursor at the very beginning of the smelt.models.users token
// ---------------------------------------------------------------------------

#[test]
fn symbol_at_cursor_path_ref_beginning() {
    let sql = "SELECT * FROM smelt.models.users\n";
    let parse = smelt_parser::parse(sql);
    let ast = AstFile::cast(parse.syntax()).expect("parses");

    // Offset pointing at the 's' of 'smelt'
    let offset = sql.find("smelt.models.users").unwrap();
    let sym = symbol_at_cursor(&ast, sql, offset);

    match sym {
        Some(SymbolAtCursor::PathRef { segments }) => {
            assert_eq!(segments, vec!["models".to_string(), "users".to_string()]);
        }
        other => panic!("expected PathRef at beginning of smelt.models.users, got {other:?}"),
    }
}
