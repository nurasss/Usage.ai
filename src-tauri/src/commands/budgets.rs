use usage_storage::Budget;
use uuid::Uuid;

use crate::appstate::AppState;
use crate::dto::BudgetDto;

#[tauri::command]
pub fn list_budgets(state: tauri::State<'_, AppState>) -> Result<Vec<BudgetDto>, String> {
    let storage = state.storage.lock().map_err(|_| "storage_lock")?;
    Ok(storage
        .list_budgets()
        .map_err(|_| "budgets_unavailable")?
        .into_iter()
        .map(|b| BudgetDto {
            account_id: b.account_id.to_string(),
            product_id: b.product_id,
            currency: b.currency,
            amount: b.amount_decimal.to_string(),
            period: b.period,
        })
        .collect())
}

#[tauri::command]
pub fn save_budget(
    account_id: String,
    product_id: String,
    currency: String,
    amount: String,
    state: tauri::State<'_, AppState>,
) -> Result<BudgetDto, String> {
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    let amount = Decimal::from_str(amount.trim()).map_err(|_| "invalid_amount")?;
    let currency = currency.trim().to_ascii_uppercase();
    let budget = Budget {
        account_id: id,
        product_id: product_id.clone(),
        currency: currency.clone(),
        amount_decimal: amount,
        period: "monthly".into(),
    };
    state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .upsert_budget(&budget)
        .map_err(|_| "invalid_budget")?;
    Ok(BudgetDto {
        account_id: id.to_string(),
        product_id,
        currency,
        amount: amount.to_string(),
        period: "monthly".into(),
    })
}

#[tauri::command]
pub fn delete_budget(
    account_id: String,
    product_id: String,
    currency: String,
    state: tauri::State<'_, AppState>,
) -> Result<bool, String> {
    let id: Uuid = account_id.parse().map_err(|_| "invalid_account_id")?;
    Ok(state
        .storage
        .lock()
        .map_err(|_| "storage_lock")?
        .delete_budget(id, &product_id, &currency.to_ascii_uppercase())
        .map_err(|_| "budget_delete_failed")?)
}
