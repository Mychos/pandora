use crate::features::generator::logic::Recipe;
use zeroize::Zeroizing;

#[derive(Default)]
pub struct Form {
    pub recipe: Recipe,
    /// The recipe `out` was forged from. The page compares the two wherever the
    /// controls sit, so the result follows a change without a frame of lag.
    pub last: Recipe,
    pub out: Zeroizing<String>,
    /// The entry the conditions modal is open for; None when it is not.
    pub entry: Option<String>,
}
