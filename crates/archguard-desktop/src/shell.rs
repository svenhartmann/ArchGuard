//! What happens when the user closes a document, closes all or quits
//! while documents have unsaved changes. Independent of Slint;
//! `shell_ui.rs` shows the prompt and carries out the result.
//!
//! Every dirty document the request affects is asked about in tab order:
//! "Save changes to <project>?" with Save, Don't save, Cancel.
//! - Save runs the document's save hook at once. If it fails, the whole
//!   request stops and the error is shown; nothing is closed.
//! - Don't save drops the changes when the request finishes.
//! - Cancel stops the whole request; nothing is closed. Documents saved
//!   before stay saved.
//!
//! When every answer is given, the request runs (close, close all, quit).

use std::collections::VecDeque;

use crate::documents::DocId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Close(DocId),
    CloseAll,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Save,
    DontSave,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Show the prompt for this document.
    Ask(DocId),
    /// Carry out the request.
    Run(Request),
    /// Stop; with the error of a failed save.
    Stop(Option<String>),
}

/// One request waiting for answers.
#[derive(Debug)]
pub struct Flow {
    request: Request,
    pending: VecDeque<DocId>,
}

impl Flow {
    /// `docs`: every open document in tab order with whether it is dirty.
    pub fn new(request: Request, docs: &[(DocId, bool)]) -> Self {
        let affected = |id: DocId| match request {
            Request::Close(c) => c == id,
            Request::CloseAll | Request::Quit => true,
        };
        let pending = docs
            .iter()
            .filter(|(id, dirty)| *dirty && affected(*id))
            .map(|(id, _)| *id)
            .collect();
        Flow { request, pending }
    }

    /// What to do now.
    pub fn next(&self) -> Next {
        match self.pending.front() {
            Some(id) => Next::Ask(*id),
            None => Next::Run(self.request),
        }
    }

    /// Apply the answer for the document [`next`](Self::next) asks about;
    /// `save` saves it.
    pub fn answer(
        &mut self,
        answer: Answer,
        save: impl FnOnce(DocId) -> Result<(), String>,
    ) -> Next {
        let Some(id) = self.pending.front().copied() else {
            return self.next();
        };
        match answer {
            Answer::Cancel => return Next::Stop(None),
            Answer::Save => {
                if let Err(e) = save(id) {
                    return Next::Stop(Some(e));
                }
            }
            Answer::DontSave => {}
        }
        self.pending.pop_front();
        self.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCS: &[(DocId, bool)] = &[(1, true), (2, false), (3, true), (4, true)];

    fn never(_: DocId) -> Result<(), String> {
        panic!("must not save")
    }

    #[test]
    fn clean_documents_run_without_a_prompt() {
        assert_eq!(
            Flow::new(Request::Close(2), DOCS).next(),
            Next::Run(Request::Close(2))
        );
        let clean = [(1, false), (2, false)];
        assert_eq!(
            Flow::new(Request::Quit, &clean).next(),
            Next::Run(Request::Quit)
        );
        assert_eq!(
            Flow::new(Request::CloseAll, &[]).next(),
            Next::Run(Request::CloseAll)
        );
    }

    #[test]
    fn closing_a_dirty_document_asks_about_it_only() {
        let mut f = Flow::new(Request::Close(3), DOCS);
        assert_eq!(f.next(), Next::Ask(3));
        assert_eq!(
            f.answer(Answer::DontSave, never),
            Next::Run(Request::Close(3))
        );
    }

    #[test]
    fn save_saves_then_runs() {
        let mut f = Flow::new(Request::Close(1), DOCS);
        let mut saved = Vec::new();
        let next = f.answer(Answer::Save, |id| {
            saved.push(id);
            Ok(())
        });
        assert_eq!(next, Next::Run(Request::Close(1)));
        assert_eq!(saved, [1]);
    }

    #[test]
    fn cancel_stops_without_saving() {
        let mut f = Flow::new(Request::Close(1), DOCS);
        assert_eq!(f.answer(Answer::Cancel, never), Next::Stop(None));
    }

    #[test]
    fn a_failed_save_stops_with_its_error() {
        let mut f = Flow::new(Request::Quit, DOCS);
        assert_eq!(
            f.answer(Answer::Save, |_| Err("disk full".into())),
            Next::Stop(Some("disk full".into()))
        );
    }

    #[test]
    fn close_all_and_quit_ask_every_dirty_document_in_tab_order() {
        for request in [Request::CloseAll, Request::Quit] {
            let mut f = Flow::new(request, DOCS);
            let mut saved = Vec::new();
            assert_eq!(f.next(), Next::Ask(1));
            let next = f.answer(Answer::Save, |id| {
                saved.push(id);
                Ok(())
            });
            assert_eq!(next, Next::Ask(3), "2 is clean");
            assert_eq!(f.answer(Answer::DontSave, never), Next::Ask(4));
            let next = f.answer(Answer::Save, |id| {
                saved.push(id);
                Ok(())
            });
            assert_eq!(next, Next::Run(request));
            assert_eq!(saved, [1, 4]);
        }
    }

    #[test]
    fn cancel_in_the_middle_of_close_all_stops_everything() {
        let mut f = Flow::new(Request::CloseAll, DOCS);
        assert_eq!(f.answer(Answer::DontSave, never), Next::Ask(3));
        assert_eq!(f.answer(Answer::Cancel, never), Next::Stop(None));
    }
}
