//! Internal consuming custody of an actual staged transaction. No caller totals,
//! public callback or reconstructed current metadata can construct source proof.
use super::*;
use photocraft_xcf::{Budget, Reservation};

mod sealed {
    pub trait Sealed {}
}

pub(in crate::xcf_import) trait PromotionSource<'b, 'a>: Sized + sealed::Sealed {
    fn original_bytes(&self) -> &[u8];
    fn document(&self) -> &Document;
    fn report_capacity_with_budget(&self, budget: &Budget<'_>) -> Result<u64, IoError>;
    fn detach_owned(self, extra: &mut [&mut Reservation<'b, 'a>], budget: &'b Budget<'a>) -> Result<staged_source::OwnedStagedImport, IoError>;
}
// The entry owns real reservations, never asserted used/peak statistics.
enum EntryFees<'b, 'a> {
    Plain { spare: Reservation<'b, 'a>, name: Reservation<'b, 'a> },
}
pub(in crate::xcf_import) struct ParsedPromotion<'b, 'a> {
    encoded: Arc<Vec<u8>>,
    completed: staged_source::CompletedStagedImport<'b, 'a>,
    fees: EntryFees<'b, 'a>,
}
impl<'b, 'a> ParsedPromotion<'b, 'a> {
    pub(in crate::xcf_import) fn plain(
        encoded: Arc<Vec<u8>>,
        completed: staged_source::CompletedStagedImport<'b, 'a>,
        spare: Reservation<'b, 'a>,
        name: Reservation<'b, 'a>,
    ) -> Self {
        Self { encoded, completed, fees: EntryFees::Plain { spare, name } }
    }
}
impl sealed::Sealed for ParsedPromotion<'_, '_> {}
impl<'b, 'a> PromotionSource<'b, 'a> for ParsedPromotion<'b, 'a> {
    fn document(&self) -> &Document {
        &self.completed.result.document
    }
    fn report_capacity_with_budget(&self, budget: &Budget<'_>) -> Result<u64, IoError> {
        owned_plain_source::census::reports(&self.completed.result, budget)
    }
    fn original_bytes(&self) -> &[u8] {
        self.encoded.as_slice()
    }
    fn detach_owned(self, extra: &mut [&mut Reservation<'b, 'a>], budget: &'b Budget<'a>) -> Result<staged_source::OwnedStagedImport, IoError> {
        struct Transfer<'b, 'a> {
            encoded: Option<Arc<Vec<u8>>>,
            completed: Option<staged_source::CompletedStagedImport<'b, 'a>>,
            fees: EntryFees<'b, 'a>,
        }
        let Self { encoded, completed, fees } = self;
        let mut transfer = Transfer { encoded: Some(encoded), completed: Some(completed), fees };
        let [model, payload, bridge] = extra else { return Err(invalid("selected source promotion receipt shape")) };
        // Checked moves only after the actual payload+fee guard exists. On any
        // later transfer error the inner transaction destroys payload first.
        let completed = transfer.completed.take().ok_or_else(|| invalid("selected completion moved"))?;
        let encoded = transfer.encoded.take().ok_or_else(|| invalid("selected encoded owner moved"))?;
        match &mut transfer.fees {
            EntryFees::Plain { spare, name } => completed.detach_owned(encoded, &mut [spare, &mut **model, &mut **payload, &mut **bridge, name], budget),
        }
    }
}
impl sealed::Sealed for staged_threshold_alpha::StagedThresholdAlphaImport<'_, '_> {}
impl<'b, 'a> PromotionSource<'b, 'a> for staged_threshold_alpha::StagedThresholdAlphaImport<'b, 'a> {
    fn document(&self) -> &Document {
        self.document()
    }
    fn report_capacity_with_budget(&self, budget: &Budget<'_>) -> Result<u64, IoError> {
        self.report_capacity_with_budget(budget)
    }
    fn original_bytes(&self) -> &[u8] {
        self.original_bytes()
    }
    fn detach_owned(self, extra: &mut [&mut Reservation<'b, 'a>], budget: &'b Budget<'a>) -> Result<staged_source::OwnedStagedImport, IoError> {
        self.detach_owned(extra, budget)
    }
}

impl sealed::Sealed for staged_selected_source::StagedSelectedImport<'_, '_> {}
impl<'b, 'a> PromotionSource<'b, 'a> for staged_selected_source::StagedSelectedImport<'b, 'a> {
    fn document(&self) -> &Document {
        self.document()
    }
    fn original_bytes(&self) -> &[u8] {
        self.original_bytes()
    }
    fn report_capacity_with_budget(&self, budget: &Budget<'_>) -> Result<u64, IoError> {
        self.report_capacity_with_budget(budget)
    }
    fn detach_owned(self, extra: &mut [&mut Reservation<'b, 'a>], budget: &'b Budget<'a>) -> Result<staged_source::OwnedStagedImport, IoError> {
        self.detach_owned(extra, budget)
    }
}
