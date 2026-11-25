use super::*;

use crate::text_editor::test_support::apply_edit;

/// The property `patch_block_preview` actually relies on: after any
/// sequence of edits (including ones that change the document's line
/// count, forcing the fallback path), the incrementally-maintained
/// `line_roles` must equal calling `classify_lines` fresh on the
/// resulting text — the "oracle" `preview/fence.rs`'s own module doc
/// now describes it as.
#[test]
fn incremental_block_preview_matches_the_oracle_after_a_hand_picked_sequence_of_edits() {
    let mut text =
        String::from("intro\n```\ncode line one\ncode line two\n```\nplain again\n> quote\n- item");
    let mut cache = DocumentCache::new(&text);
    assert_eq!(
        cache.line_roles(),
        preview::fence::classify_lines(&text).as_slice()
    );

    // (start, delete_len, insert) — a mix of same-line-count edits
    // (exercising `patch_block_preview`'s real incremental path) and
    // line-count-changing ones (exercising its fallback).
    let edits: &[(usize, usize, &str)] = &[
        (0, 0, "x"),                // insert at the very start
        (5, 0, "!"),                // insert mid-line, no newline
        (9, 0, "\n"),               // insert a bare newline
        (0, 1, ""),                 // delete one char at the start
        (0, 0, "```\n"),            // insert a new fence delimiter line
        (0, 0, "~~~\nmore\n~~~\n"), // insert a whole tilde-fenced block
    ];

    for &(start, delete_len, insert) in edits {
        let old_end = start + delete_len;
        let (new_text, edit) = apply_edit(&text, start, old_end, insert);
        cache.record_edit(&new_text, edit);
        let expected = preview::fence::classify_lines(&new_text);
        assert_eq!(
            cache.line_roles(),
            expected.as_slice(),
            "after editing {:?}",
            (start, old_end, insert)
        );
        text = new_text;
    }
}

/// Hand-picked edge cases `patch_tables` needs to get right that a
/// short random run might not stumble into: merging two previously
/// separate tables by deleting the blank line between them, splitting
/// one table into two by inserting a blank line in the middle,
/// invalidating a table entirely by corrupting its delimiter row, and
/// an edit far from any table that must stay a complete no-op.
#[test]
fn incremental_tables_handles_merge_split_and_invalidate() {
    let mut text = String::from(
        "before\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n| c | d |\n| --- | --- |\n| 3 | 4 |\n\nafter",
    );
    let mut cache = DocumentCache::new(&text);
    assert_eq!(
        cache.tables().len(),
        2,
        "sanity: two separate tables to start"
    );

    // Delete the blank line between the two tables — `classify_tables`
    // would now see one unbroken pipe-row run, but since "| 1 | 2 |"
    // isn't a valid delimiter row for a 2-column header, this merges
    // into a *single* table (the first one's body absorbing
    // everything through the second table's own rows) rather than two.
    let blank_line_start = text.find("\n\n| c").unwrap() + 1;
    let (new_text, edit) = apply_edit(&text, blank_line_start, blank_line_start + 1, "");
    cache.record_edit(&new_text, edit);
    let (expected_for_line, expected_tables) = table::classify_tables(&new_text);
    assert_eq!(table_for_line_of(&cache), expected_for_line);
    assert_eq!(cache.tables(), expected_tables.as_slice());
    assert_eq!(cache.tables().len(), 1, "the two tables merged into one");
    text = new_text;

    // Corrupt the merged table's own delimiter row so it's no longer
    // valid — the merged table is destroyed, though (per
    // `classify_tables`'s own forward-scanning algorithm) a *smaller*
    // table re-forms further down, where "| c | d |" / the second,
    // still-intact "| --- | --- |" happen to form a valid header/
    // delimiter pair of their own. The real property under test is
    // still just "matches the oracle," not a specific table count —
    // this was caught by that equality check alone, not an assumption
    // written into the test, when an earlier version of this test
    // wrongly assumed the table count would drop to zero.
    let delim_start = text.find("| --- | --- |").unwrap();
    let (new_text, edit) = apply_edit(&text, delim_start, delim_start + 1, "x");
    cache.record_edit(&new_text, edit);
    let (expected_for_line, expected_tables) = table::classify_tables(&new_text);
    assert_eq!(table_for_line_of(&cache), expected_for_line);
    assert_eq!(cache.tables(), expected_tables.as_slice());
    text = new_text;

    // An edit far from anything table-related (the leading "before")
    // must be a complete no-op for this classifier.
    let (new_text, edit) = apply_edit(&text, 0, 0, "BEFORE ");
    cache.record_edit(&new_text, edit);
    let (expected_for_line, expected_tables) = table::classify_tables(&new_text);
    assert_eq!(table_for_line_of(&cache), expected_for_line);
    assert_eq!(cache.tables(), expected_tables.as_slice());
}

/// What the removed per-line lookup arrays held, derived from `tables`
/// and `callouts`: compared against the whole-document oracles.
fn table_for_line_of(cache: &DocumentCache) -> Vec<Option<usize>> {
    (0..cache.line_count())
        .map(|i| table::table_index_at(cache.tables(), i))
        .collect()
}

fn callout_for_line_of(cache: &DocumentCache) -> Vec<Option<usize>> {
    (0..cache.line_count())
        .map(|i| preview::callout::callout_index_at(cache.callouts(), i))
        .collect()
}

/// Every field of `cache`, including the entering-state arrays and the
/// byte-offset index no public accessor exposes, must equal a cache
/// built fresh from `text`: a stale entry there could survive until a
/// later edit resumes from it.
fn assert_matches_fresh_cache(cache: &DocumentCache, text: &str, context: &str) {
    let fresh = DocumentCache::new(text);
    assert_eq!(cache.line_roles, fresh.line_roles, "line_roles {context}");
    assert_eq!(
        cache.fence_states, fresh.fence_states,
        "fence_states {context}"
    );
    assert_eq!(
        cache.block_contexts, fresh.block_contexts,
        "contexts {context}"
    );
    assert_eq!(
        cache.open_block_states, fresh.open_block_states,
        "open_block_states {context}"
    );
    assert_eq!(cache.tables, fresh.tables, "tables {context}");
    assert_eq!(
        cache.math_blocks, fresh.math_blocks,
        "math_blocks {context}"
    );
    assert_eq!(
        cache.macro_lines, fresh.macro_lines,
        "macro_lines {context}"
    );
    assert_eq!(
        cache.macros.stamp(),
        fresh.macros.stamp(),
        "macros {context}"
    );
    assert_eq!(cache.callouts, fresh.callouts, "callouts {context}");
    assert_eq!(
        cache.column_markers, fresh.column_markers,
        "column_markers {context}"
    );
    assert_eq!(cache.columns, fresh.columns, "columns {context}");
    assert_eq!(
        cache.callout_states, fresh.callout_states,
        "callout_states {context}"
    );
    assert_eq!(
        cache.line_index.len(),
        fresh.line_index.len(),
        "line_index len {context}"
    );
    for i in 0..=fresh.line_index.len() {
        assert_eq!(
            cache.line_index.prefix_sum(i),
            fresh.line_index.prefix_sum(i),
            "line_index prefix {i} {context}"
        );
    }
}

/// Same property, exercised over many pseudo-random edits — deliberate
/// coverage of positions/fragments a hand-picked sequence wouldn't
/// think to try, same "fuzz the invariant against the oracle" approach
/// `lines/fenwick.rs`'s own random test uses. Covers both `line_roles` and
/// `callout_for_line`/`callouts` together, since every edit exercises
/// both patch functions at once via the same `record_edit` call.
#[test]
fn incremental_classification_matches_the_oracle_across_many_randomized_edits() {
    for run in 0..25u64 {
        run_randomized_edits(0xC0FF_EE15_EA5E_D00Du64 ^ run.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }
}

/// A callout the user folded or unfolded keeps that state through edits in
/// its body or header and through edits above it that move its lines.
#[test]
fn fold_state_survives_edits_that_rederive_the_callout() {
    let text = "intro\n\n> [!note]- Title\n> body\n\n> [!tip]+ Open\n> body\n";
    let mut cache = DocumentCache::new(text);
    assert_eq!(
        cache
            .callouts()
            .iter()
            .map(|c| c.folded)
            .collect::<Vec<_>>(),
        [true, false]
    );
    cache.set_folded(0, false);
    cache.set_folded(1, true);
    let state = |cache: &DocumentCache| {
        cache
            .callouts()
            .iter()
            .map(|c| c.folded)
            .collect::<Vec<_>>()
    };

    let in_body = text.find("body").unwrap();
    let (typed, edit) = apply_edit(text, in_body, in_body, "x");
    cache.record_edit(&typed, edit).unwrap();
    assert_eq!(state(&cache), [false, true], "typing in a body");

    let in_title = typed.find("Title").unwrap();
    let (retitled, edit) = apply_edit(&typed, in_title, in_title, "New ");
    cache.record_edit(&retitled, edit).unwrap();
    assert_eq!(state(&cache), [false, true], "typing in a header");

    let (entered, edit) = apply_edit(&retitled, 0, 0, "\n\n");
    cache.record_edit(&entered, edit).unwrap();
    assert_eq!(state(&cache), [false, true], "lines inserted above");
    assert_eq!(cache.callouts()[0].start_line, 4);

    cache.set_folded(1, false);
    assert!(!cache.callouts()[1].folded);
}

/// Rewriting the header's marker drops the fold the user made: the file now
/// says something else. (Found by the randomized edit test.)
#[test]
fn a_rewritten_fold_marker_starts_over() {
    let text = "> [!note]- Title\n> body\n";
    let mut cache = DocumentCache::new(text);
    cache.set_folded(0, false);
    let at = text.find('-').unwrap();
    let (edited, edit) = apply_edit(text, at, at + 1, "+");
    cache.record_edit(&edited, edit).unwrap();
    assert!(!cache.callouts()[0].folded);
    let (back, edit) = apply_edit(&edited, at, at + 1, "-");
    cache.record_edit(&back, edit).unwrap();
    assert!(cache.callouts()[0].folded, "`-` says folded");
}

/// A character that removes or creates a callout used to shift every
/// later callout count, so the walk never reconverged and ran to the end
/// of the document (14ms on the 320K-line stress fixture).
#[test]
fn removing_or_creating_a_callout_does_not_walk_to_the_end_of_the_document() {
    let mut text = String::from("intro\n\n> [!note]\n> body\n\n");
    for i in 0..2000 {
        text.push_str(&format!("> [!tip]\n> body {i}\n\n"));
    }
    let mut cache = DocumentCache::new(&text);
    let callouts = cache.callouts().len();
    let blank = "intro\n\n> [!note]\n> body\n".len();

    // "x" on the blank line lazily continues the quote above, so the
    // next header is no longer a fresh blockquote start.
    let (typed, edit) = apply_edit(&text, blank, blank, "x");
    let patch = cache.record_edit(&typed, edit).unwrap();
    assert_eq!(cache.callouts().len(), callouts - 1);
    assert!(patch.touched.len() < 10, "touched {:?}", patch.touched);
    assert_matches_fresh_cache(&cache, &typed, "after removing a callout");

    let (restored, edit) = apply_edit(&typed, blank, blank + 1, "");
    let patch = cache.record_edit(&restored, edit).unwrap();
    assert_eq!(cache.callouts().len(), callouts);
    assert!(patch.touched.len() < 10, "touched {:?}", patch.touched);
    assert_matches_fresh_cache(&cache, &restored, "after restoring it");

    // An Enter before every later callout shifts them without re-walking.
    let (entered, edit) = apply_edit(&restored, 0, 0, "\n");
    let patch = cache.record_edit(&entered, edit).unwrap();
    assert!(patch.touched.len() < 10, "touched {:?}", patch.touched);
    assert_matches_fresh_cache(&cache, &entered, "after Enter at the top");
}

/// Float blocks: whatever the edits do to the markers, the blank lines and
/// the line count, the incrementally kept regions (their capped text
/// included) are the ones a fresh scan finds.
#[test]
fn float_blocks_stay_as_a_fresh_scan_finds_them_across_random_edits() {
    let mut seed = 0x5eed_f10a7_u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed >> 33
    };
    let mut text = String::from(
        "intro\n\n::: float left 30%\n\nfloat\n\n:::\n\npara one\n\npara two\n\n\
             ::: columns\n\na\n\n---\n\nb\n\n:::\n\n::: float\n\nf\n\n:::\n\nlast\n",
    );
    let mut tail = String::new();
    for i in 0..40 {
        tail.push_str(&format!("filler {i}\n\n"));
    }
    text.push_str(&tail);
    let mut cache = DocumentCache::new(&text);
    let fragments = [
        "",
        "x",
        "\n",
        "\n\n",
        "```\n",
        "text\n\n",
        ":::\n",
        "\n:::\n\n",
        "---\n",
        "::: float\n",
        "\n::: float right 20%\n\nf\n\n:::\n\n",
        "::: columns\n",
        "> [!note]\n> a\n\n",
        "::: clear\n",
        "\n::: clear\n\n",
    ];
    for step in 0..600 {
        let start = (next() as usize) % (text.len() + 1);
        let delete_len = (next() as usize) % (text.len() - start).min(12).max(1);
        let fragment = fragments[(next() as usize) % fragments.len()];
        let (new_text, edit) = apply_edit(&text, start, start + delete_len, fragment);
        cache.record_edit(&new_text, edit);
        let fresh = DocumentCache::new(&new_text);
        assert_eq!(
            cache.columns, fresh.columns,
            "step={step} edit=({start},{delete_len},{fragment:?}) text={new_text:?}"
        );
        text = new_text;
    }
}

fn run_randomized_edits(mut seed: u64) {
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        seed
    };

    let mut text = String::from(
        "intro\n```\ncode\n```\nplain\n> q\n- item\n  - nested\n    - deeper\nmore plain text here\n~~~\ntilde\n~~~\nend\n\
             > [!info]\n> body\n\n> [!warning] custom\n> more body\nafter\n\
             | a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\nafter table\n\n\
             ::: columns 60/40\n\nleft\n\n---\n\nright\n\n:::\n\n\
             ::: float left 30%\n\nfloat\n\n:::\n\npara one\n\npara two\n\npara three\n\
             $$\nx^2\n$$\n\n$$\n\\frac{a}{b}\n\n  y\n$$\n\
             $\\newcommand{\\R}{\\mathbb{R}}$ and $\\R$\n$$\n\\def\\c#1{#1}\n$$\n",
    );
    let mut cache = DocumentCache::new(&text);

    // ASCII-only, so every random byte offset below is guaranteed a
    // valid char boundary too.
    let fragments = [
        "",
        "x",
        "\n",
        "```\n",
        "~~~\n",
        "a\nb\nc",
        "```rust\ncode\n```",
        " ",
        "> [!note]\n",
        "> [!tip]+ title\n> body\n",
        "\n\n",
        "- item\n",
        "  - nested item\n",
        "    - even deeper\n",
        "> quoted\n",
        "# Heading\n",
        "| x | y |\n",
        "| --- | --- |\n",
        "|",
        " extra |",
        "\n| p | q |\n| --- | --- |\n| 5 | 6 |\n",
        "> [!note]\n> a\n> b\n\n",
        "\n\n\n\n",
        "line one\nline two\n",
        "::: columns\n",
        "\n::: columns 20/30/50\n\n",
        "---\n",
        "\n---\n\n",
        ":::\n",
        "\n:::\n\n",
        "::: columns\n\na\n\n---\n\nb\n\n:::\n",
        "::: float\n",
        "\n::: float right 20%\n\nf\n\n:::\n\n",
        "text\n\n",
        "$$",
        "$$\n",
        "\n$$\nx\n$$\n",
        "$$\na\n\nb\n$$\n",
        "x$$",
        "$\\newcommand{\\a}{x}$\n",
        "\\newcommand{\\b}[1]{#1}",
        "\n$$\n\\renewcommand{\\R}{1}\n$$\n",
        "$$\n\\DeclareMathOperator{\\Tr}{Tr}\n",
        "\\def\\d{y}",
    ];

    for step in 0..400 {
        let start = (next() as usize) % (text.len() + 1);
        let max_delete = text.len() - start;
        let delete_len = if max_delete == 0 {
            0
        } else {
            (next() as usize) % (max_delete + 1)
        };
        let fragment = fragments[(next() as usize) % fragments.len()];
        let old_end = start + delete_len;
        let (new_text, edit) = apply_edit(&text, start, old_end, fragment);
        cache.record_edit(&new_text, edit);

        let expected_roles = preview::fence::classify_lines(&new_text);
        assert_eq!(
            cache.line_roles(),
            expected_roles.as_slice(),
            "line_roles step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
        );

        let (expected_callout_for_line, expected_callouts) =
            preview::callout::classify_callouts(&new_text);
        assert_eq!(
            callout_for_line_of(&cache),
            expected_callout_for_line,
            "callout_for_line step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
        );
        assert_eq!(
            cache.callouts(),
            expected_callouts.as_slice(),
            "callouts step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
        );

        let expected_contexts = preview::classify_block_context(&new_text);
        assert_eq!(
            cache.block_contexts(),
            expected_contexts.as_slice(),
            "block_contexts step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
        );

        let (expected_table_for_line, expected_tables) = table::classify_tables(&new_text);
        assert_eq!(
            table_for_line_of(&cache),
            expected_table_for_line,
            "table_for_line step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
        );
        assert_eq!(
            cache.tables(),
            expected_tables.as_slice(),
            "tables step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"
        );

        assert_matches_fresh_cache(
            &cache,
            &new_text,
            &format!("step={step} edit=({start},{old_end},{fragment:?}) text={new_text:?}"),
        );

        text = new_text;
    }
}

/// Diagnostic, not a correctness test — measures (via `crate::test_
/// alloc`'s counting allocator, not OS-level RSS) how much of
/// `DocumentCache::new`'s own allocation cost on the 320K-line stress
/// fixture is specifically `preview::classify_block_context`'s
/// `OpenBlock::List(Vec<usize>)` tracking — every line inside an open
/// list clones the whole nesting-depth stack (`open = next.clone()` in
/// `classify_block_context_from`'s loop), a *separate heap allocation
/// per line*, not shared via `Rc`, persisted forever in `DocumentCache::
/// open_block_states`. Added after a real user report of surprisingly
/// high RSS (189MB baseline -> 3xxMB with the stress fixture open, no
/// edits made) — see HISTORY.md for the measured numbers this produced
/// and what they did/didn't explain.
#[test]
fn block_context_list_tracking_allocation_cost_on_the_stress_fixture() {
    let fixture_path = "benches/fixtures/large_markdown_stress_test.md";
    let Ok(text) = std::fs::read_to_string(fixture_path) else {
        eprintln!("stress fixture not found at {fixture_path:?}, skipping");
        return;
    };
    let line_count = text.split('\n').count();

    crate::test_alloc::reset();
    let before = crate::test_alloc::snapshot();
    let block_context_results = preview::classify_block_context_from(&text, OpenBlock::None);
    let list_lines = block_context_results
        .iter()
        .filter(|(_, state)| matches!(state, OpenBlock::List(_)))
        .count();
    let after_block_context = crate::test_alloc::snapshot();
    drop(block_context_results);

    crate::test_alloc::reset();
    let before_full = crate::test_alloc::snapshot();
    let cache = DocumentCache::new(&text);
    let after_full_retained = crate::test_alloc::snapshot();
    drop(cache);
    let after_full_dropped = crate::test_alloc::snapshot();

    eprintln!(
        "block_context_list_tracking_allocation_cost_on_the_stress_fixture: \
             {line_count} lines, {list_lines} inside an open list ({:.1}%) | \
             classify_block_context_from alone: {} allocations, peak +{} bytes ({:.1} bytes/line) | \
             full DocumentCache::new (all four classifiers): {} allocations, peak +{} bytes, \
             {} bytes still retained right after construction ({} bytes of transient garbage \
             freed once construction finished, {} bytes correctly freed on drop)",
        100.0 * list_lines as f64 / line_count as f64,
        after_block_context.count - before.count,
        after_block_context.peak - before.peak,
        (after_block_context.peak - before.peak) as f64 / line_count as f64,
        after_full_retained.count - before_full.count,
        after_full_retained.peak - before_full.peak,
        after_full_retained.current - before_full.current,
        (after_full_retained.peak - before_full.peak)
            .saturating_sub(after_full_retained.current - before_full.current),
        after_full_retained.current - after_full_dropped.current,
    );
}

const REGION: &str = "top\n\n::: columns 60/40\n\nleft\n\n---\n\nright\n\n:::\n\nbottom";

#[test]
fn the_cache_finds_a_region_and_follows_edits_around_it() {
    let mut cache = DocumentCache::new(REGION);
    assert_eq!(cache.columns().len(), 1);
    let r = cache.columns()[0];
    assert_eq!((r.start_line, r.end_line, r.count), (2, 11, 2));

    // Lines added above move it.
    let (text, edit) = apply_edit(REGION, 0, 0, "a\nb\n");
    cache.record_edit(&text, edit);
    assert_eq!(cache.columns()[0].start_line, 4);
    assert_matches_fresh_cache(&cache, &text, "lines above");

    // Breaking the blank line before the separator removes the region.
    let at = text.find("\n---").unwrap();
    let (text, edit) = apply_edit(&text, at, at + 1, "");
    cache.record_edit(&text, edit);
    assert!(cache.columns().is_empty());
    assert_matches_fresh_cache(&cache, &text, "separator joined to text");
}

#[test]
fn an_edit_in_a_column_reports_the_whole_region_as_touched() {
    let mut cache = DocumentCache::new(REGION);
    let at = REGION.find("left").unwrap();
    let (text, edit) = apply_edit(REGION, at, at, "x\ny\n");
    let patch = cache.record_edit(&text, edit).unwrap();
    let r = cache.columns()[0];
    assert!(patch.touched.start <= r.start_line && r.end_line <= patch.touched.end);
    assert_matches_fresh_cache(&cache, &text, "typing in a column");
}

#[test]
fn a_fence_opened_above_hides_the_markers_below() {
    let mut cache = DocumentCache::new(REGION);
    let (text, edit) = apply_edit(REGION, 0, 0, "```\n");
    cache.record_edit(&text, edit);
    assert!(cache.columns().is_empty());
    assert_matches_fresh_cache(&cache, &text, "fence opened");
}

#[cfg(test)]
mod line_stride_tests {
    use super::*;

    #[test]
    fn lines_of_strides_equal_split_on_newline() {
        let texts = [
            "",
            "\n",
            "\n\n",
            "a",
            "a\n",
            "\na",
            "a\n\nb",
            "a\nb\nc\n",
            "é\nü\n\n日本",
        ];
        for text in texts {
            let strides = line_strides(text);
            let lines: Vec<&str> = lines_of(text, &strides).collect();
            let expected: Vec<&str> = text.split('\n').collect();
            assert_eq!(lines, expected, "{text:?}");
            assert_eq!(strides.len(), count_lines(text), "{text:?}");
            assert_eq!(strides.iter().sum::<i64>() as usize, text.len(), "{text:?}");
        }
    }
}

/// Blocks whose pairing an edit far from them changes: deleting the blank
/// line that kept an opener unclosed, or closing a fence that was hiding one.
#[test]
fn math_blocks_follow_edits_that_reach_them_from_a_distance() {
    let mut text = String::from("$$\nx\n\n$$\nafter\n");
    let mut cache = DocumentCache::new(&text);
    assert!(cache.math_blocks().is_empty());
    // Remove the blank line: the opener now has its closer.
    let (new_text, edit) = apply_edit(&text, 5, 6, "");
    cache.record_edit(&new_text, edit);
    assert_matches_fresh_cache(&cache, &new_text, "blank removed");
    assert_eq!(cache.math_blocks().len(), 1);
    text = new_text;
    // Type a fence line above: the block is code now.
    let (new_text, edit) = apply_edit(&text, 0, 0, "```\n");
    cache.record_edit(&new_text, edit);
    assert_matches_fresh_cache(&cache, &new_text, "fence above");
    assert!(cache.math_blocks().is_empty());
    assert!(!cache.in_math_block(2));
}

#[test]
fn a_block_keeps_its_measured_rows_through_an_edit_that_does_not_touch_it() {
    let text = "top\n\n$$\nx\n$$\n\nbottom\n";
    let mut cache = DocumentCache::new(text);
    cache.math_blocks()[0].set_rows(4);
    let (new_text, edit) = apply_edit(text, 3, 3, "!");
    cache.record_edit(&new_text, edit);
    assert_eq!(cache.math_blocks()[0].rows(), Some(4));
    // An edit inside it drops the measure: the row cache takes it again.
    let (inside, edit) = apply_edit(&new_text, 9, 9, "y");
    let patch = cache.record_edit(&inside, edit).unwrap();
    assert_eq!(cache.math_blocks()[0].rows(), None);
    assert!(patch.touched.contains(&2) && patch.touched.contains(&4));
}

#[test]
fn a_definition_edit_dirties_the_whole_note_and_other_edits_do_not() {
    let text = "$\\newcommand{\\R}{x}$\n\nfar\n\nlet $\\R$\n\nmore\n";
    let mut cache = DocumentCache::new(text);
    assert!(!cache.macros().is_empty());
    let before = cache.macros().stamp();

    let far = text.find("far").unwrap() + 1;
    let (edited, edit) = apply_edit(text, far, far, "!");
    let patch = cache.record_edit(&edited, edit).unwrap();
    assert!(
        patch.touched.len() < cache.line_count(),
        "{:?}",
        patch.touched
    );
    assert_eq!(cache.macros().stamp(), before);

    let at = edited.find('x').unwrap();
    let (changed, edit) = apply_edit(&edited, at, at + 1, "y");
    let patch = cache.record_edit(&changed, edit).unwrap();
    assert_eq!(patch.touched, 0..cache.line_count());
    assert_ne!(cache.macros().stamp(), before);
    assert_matches_fresh_cache(&cache, &changed, "definition changed");

    let (gone, edit) = apply_edit(&changed, 0, changed.find('\n').unwrap(), "");
    let patch = cache.record_edit(&gone, edit).unwrap();
    assert_eq!(patch.touched, 0..cache.line_count());
    assert!(cache.macros().is_empty());
}
