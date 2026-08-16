/// Upper bound on a wikilink target's byte length. A real target is a short
/// title (optionally id-bound), never a paragraph; anything longer is a `[[`
/// opened in prose or an inline-code span whose `]]` only appears far away.
/// Bounding it also keeps the value below Postgres' btree index row limit, so a
/// false positive cannot overflow the unique index on `document_links`.
const MAX_WIKILINK_TARGET_LEN: usize = 512;

/// A syntactically valid comment-link input that still needs workspace-scoped
/// classification before it becomes a derived link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentLinkCandidate {
    /// `[[task:ATL-80]]` written in a comment.
    Task {
        readable_id: String,
    },
    /// `[[note:incident-runbook]]` written in a comment.
    Note {
        slug: String,
    },
    /// The id-bound `[[<uuid>|Title]]` form, which may address either kind.
    Uuid(uuid::Uuid),
    AttachmentUrl(CommentAttachmentUrl),
}

/// Canonical root-relative attachment location carried without resolving it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentAttachmentUrl {
    pub workspace_slug: String,
    pub owner: CommentAttachmentUrlOwner,
    pub comment_id: uuid::Uuid,
    pub attachment_id: uuid::Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentAttachmentUrlOwner {
    Task { readable_id: String },
    Document { slug: String },
}

/// What a `[[…]]` wikilink points at, once its inner text is classified.
///
/// The typed forms address their target by the identifier a reader already
/// knows — a task's readable id, a note's slug, an attachment's file name — so
/// no UUID has to appear in the markdown a person edits. The two untyped forms
/// are the shapes that predate typed links and keep their exact meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WikilinkTarget {
    /// `[[task:ATL-80]]`, addressed by readable id.
    Task { readable_id: String },
    /// `[[note:incident-runbook]]`, addressed by document slug.
    ///
    /// A slug is assigned once at document creation and never regenerated, so
    /// this address survives renaming the note.
    Note { slug: String },
    /// `[[file:policy.pdf]]`, addressed by file name among the attachments of
    /// whichever document or task contains the link.
    File { file_name: String },
    /// `[[<uuid>|Title]]`: a document bound to its id.
    Document { id: uuid::Uuid },
    /// `[[Title]]`: a document addressed by the slug of the given title.
    Title,
}

/// A classified wikilink: where it points and what a reader sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedWikilink {
    pub target: WikilinkTarget,
    /// The display half after `|`, or the address itself when there is none.
    ///
    /// For an untyped link this is the title the link is resolved by, which is
    /// why the two coincide there.
    pub display: String,
}

/// Classifies the inner text of a `[[…]]` token into a target and a display
/// string.
///
/// The text is split on the FIRST `|`. A `kind:address` left half with a known
/// kind produces a typed target; everything else falls through to the untyped
/// rules unchanged, which is what keeps every previously written wikilink
/// meaning exactly what it meant before.
pub fn classify_wikilink(raw: &str) -> ParsedWikilink {
    let (left, display) = match raw.split_once('|') {
        Some((left, right)) => (left.trim(), Some(right.trim())),
        None => (raw.trim(), None),
    };

    if let Some((target, address)) = typed_target(left) {
        return ParsedWikilink {
            target,
            display: display.unwrap_or(address).to_string(),
        };
    }

    if let Some(display) = display
        && let Ok(id) = uuid::Uuid::parse_str(left)
    {
        return ParsedWikilink {
            target: WikilinkTarget::Document { id },
            display: display.to_string(),
        };
    }

    ParsedWikilink {
        target: WikilinkTarget::Title,
        display: raw.trim().to_string(),
    }
}

/// Splits a `kind:address` left half, returning the target and the raw address.
///
/// A colon followed by whitespace is prose — a document titled `task: rewrite
/// the parser` must stay a title link — so the kind only binds when an address
/// starts immediately after the colon.
fn typed_target(left: &str) -> Option<(WikilinkTarget, &str)> {
    let (kind, address) = left.split_once(':')?;

    if address.starts_with(char::is_whitespace) {
        return None;
    }

    let address = address.trim_end();
    if address.is_empty() {
        return None;
    }

    let target = match kind.to_ascii_lowercase().as_str() {
        // Readable ids are stored uppercase and matched exactly, so `atl-80`
        // typed by hand has to be normalized here to resolve.
        "task" => WikilinkTarget::Task {
            readable_id: address.to_ascii_uppercase(),
        },
        "note" => WikilinkTarget::Note {
            slug: address.to_string(),
        },
        "file" => WikilinkTarget::File {
            file_name: address.to_string(),
        },
        _ => return None,
    };

    Some((target, address))
}

/// Parses `[[Target]]` wikilinks from markdown content.
///
/// Returns de-duplicated target strings in order of first appearance.
/// Multi-line or oversized candidates are skipped: a genuine wikilink is a
/// short, single-line title, so such spans are false positives.
pub fn parse_wikilinks(content: &str) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    let mut remaining = content;

    while let Some(open) = remaining.find("[[") {
        remaining = &remaining[open + 2..];

        if let Some(close) = remaining.find("]]") {
            let target = &remaining[..close];
            remaining = &remaining[close + 2..];

            if !target.is_empty()
                && !target.contains('\n')
                && target.len() <= MAX_WIKILINK_TARGET_LEN
                && !targets.iter().any(|t| t == target)
            {
                targets.push(target.to_string());
            }
        } else {
            break;
        }
    }

    targets
}

/// Resolves the inner content of a `[[...]]` wikilink into an optional target
/// document id and a display title.
///
/// The syntax is split on the FIRST `|`. When a `|` is present and the left part
/// (trimmed) parses as a UUID, the link is id-bound: the UUID is the stable
/// target and the right part (trimmed) is the display title. Otherwise the link
/// is treated as legacy / hand-typed: no id, and the whole trimmed `raw` is the
/// title — this deliberately covers a `|` whose left part is not a UUID (e.g.
/// `Foo|Bar`), which stays a legacy title rather than an id-bound link.
pub fn parse_wikilink_target(raw: &str) -> (Option<uuid::Uuid>, String) {
    if let Some((left, right)) = raw.split_once('|')
        && let Ok(id) = uuid::Uuid::parse_str(left.trim())
    {
        return (Some(id), right.trim().to_string());
    }

    (None, raw.trim().to_string())
}

/// Extracts comment-link inputs without mutating Markdown.
///
/// Typed `task:` and `note:` wikilinks become their own candidates; the
/// id-bound form stays a UUID candidate, which persistence resolves against
/// either kind. A title-only `[[Title]]` is deliberately not a candidate: in a
/// comment it reads as prose, and guessing a target from it would link things
/// nobody asked to link.
///
/// Attachment URLs must be canonical root-relative Atlas routes; resolving
/// their workspace, parent, comment, and ownership chain is intentionally
/// deferred to persistence.
pub fn parse_comment_link_candidates(content: &str) -> Vec<CommentLinkCandidate> {
    let mut candidates = Vec::new();

    for raw in parse_wikilinks(content) {
        let candidate = match classify_wikilink(&raw).target {
            WikilinkTarget::Task { readable_id } => CommentLinkCandidate::Task { readable_id },
            WikilinkTarget::Note { slug } => CommentLinkCandidate::Note { slug },
            WikilinkTarget::Document { id } => CommentLinkCandidate::Uuid(id),
            WikilinkTarget::File { .. } | WikilinkTarget::Title => continue,
        };

        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }

    for url in markdown_urls(content) {
        let Some(attachment) = parse_comment_attachment_url(url) else {
            continue;
        };

        let candidate = CommentLinkCandidate::AttachmentUrl(attachment);
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }

    candidates
}

fn markdown_urls(content: &str) -> impl Iterator<Item = &str> {
    content
        .split("](")
        .skip(1)
        .filter_map(|remainder| remainder.split_once(')').map(|(url, _)| url))
}

fn parse_comment_attachment_url(url: &str) -> Option<CommentAttachmentUrl> {
    if !url.starts_with('/') || url.contains(['?', '#', '%']) {
        return None;
    }

    let segments = url.split('/').collect::<Vec<_>>();
    if segments
        .iter()
        .skip(1)
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return None;
    }

    let [
        "",
        "api",
        "workspaces",
        workspace_slug,
        parent_kind,
        parent_value,
        "comments",
        comment_id,
        "attachments",
        attachment_id,
        suffix @ ..,
    ] = segments.as_slice()
    else {
        return None;
    };

    if !is_canonical_segment(workspace_slug) {
        return None;
    }

    let owner = match (*parent_kind, *parent_value, suffix) {
        ("tasks", readable_id, ["content"]) if is_canonical_segment(readable_id) => {
            CommentAttachmentUrlOwner::Task {
                readable_id: readable_id.to_string(),
            }
        }
        ("documents", slug, []) if is_canonical_segment(slug) => {
            CommentAttachmentUrlOwner::Document {
                slug: slug.to_string(),
            }
        }
        _ => return None,
    };

    let comment_id = parse_canonical_uuid(comment_id)?;
    let attachment_id = parse_canonical_uuid(attachment_id)?;

    Some(CommentAttachmentUrl {
        workspace_slug: (*workspace_slug).to_string(),
        owner,
        comment_id,
        attachment_id,
    })
}

fn is_canonical_segment(value: &str) -> bool {
    !value.is_empty() && !value.contains(['?', '#', '%', '/'])
}

fn parse_canonical_uuid(value: &str) -> Option<uuid::Uuid> {
    let id = uuid::Uuid::parse_str(value).ok()?;
    (id.to_string() == value).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_candidates_keep_uuid_wikilinks_verbatim_and_deduplicate_attachment_urls() {
        let document_id = uuid::Uuid::now_v7();
        let attachment_id = uuid::Uuid::now_v7();
        let comment_id = uuid::Uuid::now_v7();
        let content = format!(
            "[[{document_id}|Renamed later]] [file](/api/workspaces/acme/tasks/ATL-42/comments/{comment_id}/attachments/{attachment_id}/content) [same](/api/workspaces/acme/tasks/ATL-42/comments/{comment_id}/attachments/{attachment_id}/content)"
        );

        assert_eq!(
            parse_comment_link_candidates(&content),
            vec![
                CommentLinkCandidate::Uuid(document_id),
                CommentLinkCandidate::AttachmentUrl(CommentAttachmentUrl {
                    workspace_slug: "acme".into(),
                    owner: CommentAttachmentUrlOwner::Task {
                        readable_id: "ATL-42".into(),
                    },
                    comment_id,
                    attachment_id,
                }),
            ]
        );
    }

    #[test]
    fn comment_candidates_carry_typed_task_and_note_links() {
        let content = "blocked by [[task:ATL-80|the login bug]], see [[note:incident-runbook]]";

        assert_eq!(
            parse_comment_link_candidates(content),
            vec![
                CommentLinkCandidate::Task {
                    readable_id: "ATL-80".into()
                },
                CommentLinkCandidate::Note {
                    slug: "incident-runbook".into()
                },
            ]
        );
    }

    #[test]
    fn comment_candidates_deduplicate_repeated_typed_links() {
        let content = "[[task:ATL-80]] and again [[task:ATL-80|same task]] and [[task:atl-80]]";

        assert_eq!(
            parse_comment_link_candidates(content),
            vec![CommentLinkCandidate::Task {
                readable_id: "ATL-80".into()
            }]
        );
    }

    #[test]
    fn comment_candidates_ignore_title_only_and_file_links() {
        // A bare `[[Title]]` in a comment reads as prose; guessing a target from
        // it would link resources nobody asked to link. `file:` is addressed by
        // the attachment URL form instead.
        let content = "[[Some Title]] and [[file:policy.pdf]]";

        assert_eq!(
            parse_comment_link_candidates(content),
            Vec::<CommentLinkCandidate>::new()
        );
    }

    #[test]
    fn comment_candidates_reject_noncanonical_attachment_urls() {
        let attachment_id = uuid::Uuid::now_v7();
        let comment_id = uuid::Uuid::now_v7();
        let valid_document = format!(
            "/api/workspaces/acme/documents/roadmap/comments/{comment_id}/attachments/{attachment_id}"
        );
        let uppercase = attachment_id.to_string().to_uppercase();
        let invalid = format!(
            "[query]({valid_document}?download=1) [absolute](https://atlas.test{valid_document}) [uppercase](/api/workspaces/acme/tasks/ATL-42/comments/{comment_id}/attachments/{uppercase}/content)"
        );

        assert_eq!(
            parse_comment_link_candidates(&format!("[valid]({valid_document}) {invalid}")),
            vec![CommentLinkCandidate::AttachmentUrl(CommentAttachmentUrl {
                workspace_slug: "acme".into(),
                owner: CommentAttachmentUrlOwner::Document {
                    slug: "roadmap".into(),
                },
                comment_id,
                attachment_id,
            })]
        );
    }

    #[test]
    fn empty_content_returns_empty() {
        assert_eq!(parse_wikilinks(""), Vec::<String>::new());
    }

    #[test]
    fn no_wikilinks_returns_empty() {
        assert_eq!(
            parse_wikilinks("Just some plain text with [single] brackets."),
            Vec::<String>::new()
        );
    }

    #[test]
    fn single_wikilink_extracted() {
        assert_eq!(
            parse_wikilinks("See [[Architecture]] for details."),
            vec!["Architecture".to_string()]
        );
    }

    #[test]
    fn duplicate_wikilinks_deduplicated() {
        let result = parse_wikilinks("[[Foo]] and then [[Foo]] again");
        assert_eq!(result, vec!["Foo".to_string()]);
    }

    #[test]
    fn multiple_distinct_wikilinks_in_order_of_first_appearance() {
        let result = parse_wikilinks("[[Alpha]] then [[Beta]] then [[Alpha]] then [[Gamma]]");
        assert_eq!(
            result,
            vec!["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()]
        );
    }

    #[test]
    fn malformed_unclosed_bracket_not_matched() {
        assert_eq!(parse_wikilinks("[[Unclosed"), Vec::<String>::new());
    }

    #[test]
    fn multiline_content_parsed_correctly() {
        let content = "# Title\n\nSee [[Design Doc]] for details.\n\nAlso check [[Architecture]].";
        let result = parse_wikilinks(content);
        assert_eq!(
            result,
            vec!["Design Doc".to_string(), "Architecture".to_string()]
        );
    }

    #[test]
    fn empty_wikilink_not_included() {
        assert_eq!(parse_wikilinks("[[]]"), Vec::<String>::new());
    }

    #[test]
    fn multiline_target_is_ignored() {
        // A real wikilink is single-line; a `[[` opened in prose/code that only
        // closes lines later must not be captured as a target.
        assert_eq!(
            parse_wikilinks("[[line one\nline two]]"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn oversized_target_is_ignored() {
        let big = "x".repeat(600);
        assert_eq!(parse_wikilinks(&format!("[[{big}]]")), Vec::<String>::new());
    }

    #[test]
    fn oversized_target_does_not_block_later_links() {
        let big = "x".repeat(600);
        assert_eq!(
            parse_wikilinks(&format!("[[{big}]] and [[Real]]")),
            vec!["Real".to_string()]
        );
    }

    #[test]
    fn target_id_bound_link_returns_uuid_and_display_title() {
        let uuid =
            uuid::Uuid::parse_str("019ed5fa-0000-7000-8000-000000000000").expect("valid test uuid");
        let raw = format!("{uuid}|Editor test");

        assert_eq!(
            parse_wikilink_target(&raw),
            (Some(uuid), "Editor test".to_string())
        );
    }

    #[test]
    fn target_id_bound_link_trims_surrounding_whitespace() {
        let uuid =
            uuid::Uuid::parse_str("019ed5fa-0000-7000-8000-000000000000").expect("valid test uuid");
        let raw = format!("  {uuid}  |  Editor test  ");

        assert_eq!(
            parse_wikilink_target(&raw),
            (Some(uuid), "Editor test".to_string())
        );
    }

    #[test]
    fn target_plain_title_returns_none_and_title() {
        assert_eq!(
            parse_wikilink_target("Plain Title"),
            (None, "Plain Title".to_string())
        );
    }

    #[test]
    fn target_non_uuid_before_pipe_is_legacy_title() {
        assert_eq!(
            parse_wikilink_target("Foo|Bar"),
            (None, "Foo|Bar".to_string())
        );
    }

    #[test]
    fn target_empty_returns_none_and_empty_title() {
        assert_eq!(parse_wikilink_target(""), (None, String::new()));
    }

    #[test]
    fn classify_typed_task_uses_the_readable_id_as_display_without_a_pipe() {
        assert_eq!(
            classify_wikilink("task:ATL-80"),
            ParsedWikilink {
                target: WikilinkTarget::Task {
                    readable_id: "ATL-80".into()
                },
                display: "ATL-80".into(),
            }
        );
    }

    #[test]
    fn classify_typed_link_prefers_the_display_half() {
        assert_eq!(
            classify_wikilink("  note:incident-runbook  |  the runbook  "),
            ParsedWikilink {
                target: WikilinkTarget::Note {
                    slug: "incident-runbook".into()
                },
                display: "the runbook".into(),
            }
        );
    }

    #[test]
    fn classify_normalizes_the_kind_and_the_readable_id_case() {
        assert_eq!(
            classify_wikilink("TASK:atl-80"),
            ParsedWikilink {
                target: WikilinkTarget::Task {
                    readable_id: "ATL-80".into()
                },
                display: "atl-80".into(),
            }
        );
    }

    #[test]
    fn classify_keeps_file_names_verbatim() {
        assert_eq!(
            classify_wikilink("file:Q3 policy.pdf|the policy"),
            ParsedWikilink {
                target: WikilinkTarget::File {
                    file_name: "Q3 policy.pdf".into()
                },
                display: "the policy".into(),
            }
        );
    }

    #[test]
    fn classify_leaves_a_title_that_merely_starts_with_a_kind_word_alone() {
        // The space after the colon is what separates prose from an address.
        assert_eq!(
            classify_wikilink("task: rewrite the parser"),
            ParsedWikilink {
                target: WikilinkTarget::Title,
                display: "task: rewrite the parser".into(),
            }
        );
    }

    #[test]
    fn classify_leaves_an_unknown_kind_alone() {
        assert_eq!(
            classify_wikilink("http://example.test"),
            ParsedWikilink {
                target: WikilinkTarget::Title,
                display: "http://example.test".into(),
            }
        );
    }

    #[test]
    fn classify_rejects_a_kind_with_no_address() {
        assert_eq!(
            classify_wikilink("note:"),
            ParsedWikilink {
                target: WikilinkTarget::Title,
                display: "note:".into(),
            }
        );
    }

    #[test]
    fn classify_keeps_the_id_bound_document_form() {
        let id =
            uuid::Uuid::parse_str("019ed5fa-0000-7000-8000-000000000000").expect("valid test uuid");

        assert_eq!(
            classify_wikilink(&format!("  {id}  |  Editor test  ")),
            ParsedWikilink {
                target: WikilinkTarget::Document { id },
                display: "Editor test".into(),
            }
        );
    }

    #[test]
    fn classify_keeps_a_bare_uuid_without_a_pipe_as_a_title() {
        // `parse_wikilink_target` only binds an id when a display half follows,
        // and the classifier must not widen that.
        let id = uuid::Uuid::now_v7().to_string();

        assert_eq!(
            classify_wikilink(&id),
            ParsedWikilink {
                target: WikilinkTarget::Title,
                display: id,
            }
        );
    }

    #[test]
    fn classify_keeps_a_non_uuid_before_a_pipe_as_a_title() {
        assert_eq!(
            classify_wikilink("Foo|Bar"),
            ParsedWikilink {
                target: WikilinkTarget::Title,
                display: "Foo|Bar".into(),
            }
        );
    }
}
