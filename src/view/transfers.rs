//! The transfers pane: every copy, upload and download, newest first, with cancel.

use rune::{El, ScrollSpec, col, row, text};
use rune_kit::{Button, ButtonVariant, IconButton, IconButtonVariant, Kit, Spinner, glyphs};

use crate::app::App;
use crate::icons;
use crate::jobs::{Direction, Job, JobState};

pub fn build(s: &App, k: &Kit) -> El {
    let jobs: Vec<&Job> = s.jobs.list.iter().filter(|j| j.transfer).rev().collect();
    let finished = jobs.iter().any(|j| j.state.finished());
    let header = row().w_full().h(36.0).px(12.0).center_y().justify_between().child(text("Transfers").font(k.ty.ui_medium).color(k.c.text_base)).child(
        Button::new("Clear")
            .variant(ButtonVariant::GhostMuted)
            .disabled(!finished)
            .on_press(|s: &mut App| s.jobs.clear_finished())
            .build(k)
            .key("clear-transfers"),
    );
    let body = if jobs.is_empty() {
        col()
            .w_full()
            .padding(16.0)
            .gap(6.0)
            .child(text("No transfers yet").font(k.ty.ui).color(k.c.text_faint))
            .child(text("Copy files on one machine and paste them on another: the copy runs over scp.").font(k.ty.small).color(k.c.text_faint).wrap())
    } else {
        jobs.into_iter().fold(col().w_full().px(8.0).gap(2.0), |c, j| c.child(job_row(j, k)))
    };
    col()
        .key("transfers")
        .w_full()
        .h_full()
        .min_h(0.0)
        .child(header)
        .child(col().w_full().grow(1.0).min_h(0.0).scroll(ScrollSpec { stick_bottom: false }).child(body))
}

fn job_row(j: &Job, k: &Kit) -> El {
    let glyph = match j.direction {
        Direction::Upload => &icons::UPLOAD,
        Direction::Download => &icons::DOWNLOADS,
        Direction::Local | Direction::Remote => &glyphs::COPY,
    };
    let (status, ink): (El, _) = match &j.state {
        JobState::Queued => (text("Waiting").font(k.ty.text_small).color(k.c.text_faint), k.c.text_base),
        JobState::Running => (
            row().gap(6.0).center_y().child(Spinner::new().size(11.0).build(k)).child(text("Copying over scp…").font(k.ty.text_small).color(k.c.text_muted)),
            k.c.text_base,
        ),
        JobState::Done => (text("Done").font(k.ty.text_small).color(k.c.success.icon), k.c.text_base),
        JobState::Cancelled => (text("Cancelled").font(k.ty.text_small).color(k.c.text_faint), k.c.text_muted),
        JobState::Failed(e) => (text(e.clone()).font(k.ty.text_small).color(k.c.danger.icon).wrap(), k.c.text_base),
    };
    let id = j.id;
    let mut r =
        row().key(format!("job-{id}")).w_full().px(8.0).py(8.0).gap(10.0).radius(k.radius.md).child(rune::icon(glyph, 16.0).color(k.c.icon_muted)).child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(if j.state == JobState::Done { j.done_title.clone().unwrap_or_else(|| j.title.clone()) } else { j.title.clone() })
                        .font(k.ty.ui)
                        .color(ink)
                        .truncate(),
                )
                .child(text(j.detail.clone()).font(k.ty.text_small).color(k.c.text_faint).truncate())
                .child(status),
        );
    if !j.state.finished() {
        r = r.child(
            IconButton::new(&glyphs::CLOSE)
                .variant(IconButtonVariant::GhostMuted)
                .label("Cancel")
                .help("Cancel")
                .on_press(move |s: &mut App| s.jobs.cancel(id))
                .build(k)
                .key(format!("cancel-{id}")),
        );
    }
    r
}
