//! A unified diff as flat rows, ready for a uniform list: one row per file header, hunk header
//! and line. Plain text with three colours; syntax highlighting is v0.2 (§5.3).

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    File {
        path: String,
        additions: usize,
        deletions: usize,
    },
    Hunk(String),
    Added(String),
    Removed(String),
    Context(String),
}

pub fn parse(diff: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut file: Option<usize> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // "a/x b/x"; new files from `--no-index` read "a/dev/null b/x".
            let path = rest.rsplit(" b/").next().unwrap_or(rest).to_owned();
            file = Some(rows.len());
            rows.push(Row::File {
                path,
                additions: 0,
                deletions: 0,
            });
        } else if line.starts_with("@@") {
            rows.push(Row::Hunk(line.to_owned()));
        } else if line.starts_with("+++") || line.starts_with("---") || file.is_none() {
            // File markers repeat the header; anything before the first file is noise.
        } else if let Some(text) = line.strip_prefix('+') {
            bump(&mut rows, file, true);
            rows.push(Row::Added(text.to_owned()));
        } else if let Some(text) = line.strip_prefix('-') {
            bump(&mut rows, file, false);
            rows.push(Row::Removed(text.to_owned()));
        } else if let Some(text) = line.strip_prefix(' ') {
            rows.push(Row::Context(text.to_owned()));
        }
        // "index …", "new file mode …", "\ No newline at end of file": not shown.
    }
    rows
}

fn bump(rows: &mut [Row], file: Option<usize>, added: bool) {
    if let Some(Row::File {
        additions,
        deletions,
        ..
    }) = file.and_then(|i| rows.get_mut(i))
    {
        if added {
            *additions += 1;
        } else {
            *deletions += 1;
        }
    }
}

/// Rows with the files the user marked as viewed folded down to their header.
pub fn visible<'a>(rows: &'a [Row], viewed: &dyn Fn(&str) -> bool) -> Vec<&'a Row> {
    let mut out = Vec::new();
    let mut hiding = false;
    for row in rows {
        if let Row::File { path, .. } = row {
            hiding = viewed(path);
            out.push(row);
        } else if !hiding {
            out.push(row);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,2 +1,2 @@\n fn a() {}\n-old\n+new\n+more\ndiff --git a/dev/null b/notes.txt\nnew file mode 100644\n--- /dev/null\n+++ b/notes.txt\n@@ -0,0 +1 @@\n+hi\n\\ No newline at end of file";

    #[test]
    fn files_hunks_and_lines() {
        let rows = parse(DIFF);
        assert_eq!(
            rows[0],
            Row::File {
                path: "src/a.rs".into(),
                additions: 2,
                deletions: 1
            }
        );
        assert_eq!(rows[1], Row::Hunk("@@ -1,2 +1,2 @@".into()));
        assert_eq!(rows[2], Row::Context("fn a() {}".into()));
        assert_eq!(rows[3], Row::Removed("old".into()));
        assert_eq!(rows[5], Row::Added("more".into()));
        assert_eq!(
            rows[6],
            Row::File {
                path: "notes.txt".into(),
                additions: 1,
                deletions: 0
            }
        );
        assert_eq!(rows.len(), 9);
    }

    #[test]
    fn viewed_files_fold_to_their_header() {
        let rows = parse(DIFF);
        let shown = visible(&rows, &|path| path == "src/a.rs");
        assert_eq!(
            shown.len(),
            4,
            "a.rs header, then notes.txt header, hunk and line"
        );
    }
}
