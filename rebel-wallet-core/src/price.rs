use serde::Deserialize;

use crate::PriceCurrency;

#[derive(Deserialize)]
struct PriceResponse {
    price: f64,
}

pub(crate) async fn fetch_bitcoin_price(currency: &PriceCurrency) -> anyhow::Result<f64> {
    if currency == &PriceCurrency::BTC {
        return Ok(1.0);
    }

    let currency = currency.code();
    let url = format!("https://price.rebelwallet.app/price/{currency}");
    let response: PriceResponse = reqwest::get(url).await?.error_for_status()?.json().await?;
    if !response.price.is_finite() {
        anyhow::bail!("invalid BTC/{currency} price");
    }
    Ok(response.price)
}
