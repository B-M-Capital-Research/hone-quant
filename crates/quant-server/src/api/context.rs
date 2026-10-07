//! Which portfolio a request acts on.
//!
//! The web app sends the selected portfolio in `X-Hone-Quant-Portfolio` (or `?portfolio=` where
//! a header cannot be set: EventSource, links). Without it the user's default portfolio applies:
//! the first visible active one, shared portfolios first. Visibility and the right to act come
//! from [`CurrentUser::can_view`] and [`CurrentUser::can_trade`].

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use super::error::ApiError;
use crate::auth::{CurrentUser, Role};
use crate::state::SharedState;
use crate::store::portfolios::{self, Book, Portfolio};
use crate::store::trading::{self, Account};

pub const PORTFOLIO_HEADER: &str = "x-hone-quant-portfolio";

/// The portfolio a request acts on, with its active account.
pub struct PortfolioCtx {
    pub user: CurrentUser,
    pub book: Book,
}

impl PortfolioCtx {
    pub fn portfolio(&self) -> &Portfolio {
        &self.book.portfolio
    }

    pub fn account(&self) -> &Account {
        &self.book.account
    }

    pub fn can_trade(&self) -> bool {
        self.user.can_trade(&self.book.portfolio)
    }
}

/// [`PortfolioCtx`] for requests that change the portfolio: the user must be allowed to act on it.
pub struct TradeCtx(pub PortfolioCtx);

/// For read models that also make sense without a portfolio (the market status bar).
pub struct MaybePortfolio {
    pub user: CurrentUser,
    pub book: Option<Book>,
}

/// The id in the header or the `portfolio` query parameter.
fn requested(parts: &Parts) -> Result<Option<i64>, ApiError> {
    let raw = parts
        .headers
        .get(PORTFOLIO_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            parts.uri.query().and_then(|query| {
                query
                    .split('&')
                    .filter_map(|pair| pair.split_once('='))
                    .find(|(key, _)| *key == "portfolio")
                    .map(|(_, value)| value.to_string())
            })
        });
    match raw.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(value) => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| ApiError::bad("the portfolio id must be a number")),
    }
}

/// Resolves the requested (or default) portfolio for `user`.
pub async fn resolve(
    state: &SharedState,
    user: &CurrentUser,
    requested: Option<i64>,
) -> Result<Book, ApiError> {
    let client = state.pool.get().await?;
    let portfolio = match requested {
        Some(id) => portfolios::get(&client, id)
            .await?
            .filter(|p| p.is_active() && user.can_view(p))
            .ok_or(ApiError::PortfolioNotFound)?,
        None => portfolios::list(&client, false)
            .await?
            .into_iter()
            .find(|p| user.can_view(p))
            .ok_or(ApiError::NoPortfolio)?,
    };
    let account = trading::active_account(&client, portfolio.id)
        .await?
        .ok_or(ApiError::PortfolioNotFound)?;
    Ok(Book { portfolio, account })
}

/// The portfolios whose notifications and events `user` may see: `None` = all of them.
pub async fn visible_ids(
    state: &SharedState,
    user: &CurrentUser,
) -> Result<Option<Vec<i64>>, ApiError> {
    if user.role != Role::Member {
        return Ok(None);
    }
    let client = state.pool.get().await?;
    Ok(Some(
        portfolios::list(&client, true)
            .await?
            .into_iter()
            .filter(|p| user.can_view(p))
            .map(|p| p.id)
            .collect(),
    ))
}

impl FromRequestParts<SharedState> for PortfolioCtx {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        let book = resolve(state, &user, requested(parts)?).await?;
        Ok(PortfolioCtx { user, book })
    }
}

impl FromRequestParts<SharedState> for TradeCtx {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let ctx = PortfolioCtx::from_request_parts(parts, state).await?;
        if !ctx.can_trade() {
            return Err(ApiError::Forbidden(
                "you cannot act on this portfolio".into(),
            ));
        }
        Ok(TradeCtx(ctx))
    }
}

impl FromRequestParts<SharedState> for MaybePortfolio {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        let book = match resolve(state, &user, requested(parts)?).await {
            Ok(book) => Some(book),
            Err(ApiError::NoPortfolio) => None,
            Err(error) => return Err(error),
        };
        Ok(MaybePortfolio { user, book })
    }
}

/// Checks that `user` may see (and, with `act`, act on) the portfolio an account belongs to —
/// for resources addressed by id (plans, orders), whatever the selected portfolio is.
pub async fn authorize_account(
    state: &SharedState,
    user: &CurrentUser,
    account_id: i64,
    act: bool,
) -> Result<Portfolio, ApiError> {
    let client = state.pool.get().await?;
    let portfolio = portfolios::for_account(&client, account_id).await?;
    if !user.can_view(&portfolio) {
        return Err(ApiError::not_found("plan"));
    }
    if act && !user.can_trade(&portfolio) {
        return Err(ApiError::Forbidden(
            "you cannot act on this portfolio".into(),
        ));
    }
    Ok(portfolio)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    fn parts(uri: &str, header: Option<&str>) -> Parts {
        let mut builder = Request::builder().uri(uri);
        if let Some(value) = header {
            builder = builder.header(PORTFOLIO_HEADER, value);
        }
        builder.body(()).unwrap().into_parts().0
    }

    #[test]
    fn the_header_wins_over_the_query_and_bad_ids_are_refused() {
        assert_eq!(requested(&parts("/api/plans", None)).unwrap(), None);
        assert_eq!(requested(&parts("/api/plans", Some("7"))).unwrap(), Some(7));
        assert_eq!(
            requested(&parts("/api/events?portfolio=3", None)).unwrap(),
            Some(3)
        );
        assert_eq!(
            requested(&parts("/api/events?a=1&portfolio=3", Some("5"))).unwrap(),
            Some(5)
        );
        assert_eq!(requested(&parts("/api/plans", Some(" "))).unwrap(), None);
        assert!(requested(&parts("/api/plans", Some("x"))).is_err());
        assert!(requested(&parts("/api/plans?portfolio=1;drop", None)).is_err());
    }
}
