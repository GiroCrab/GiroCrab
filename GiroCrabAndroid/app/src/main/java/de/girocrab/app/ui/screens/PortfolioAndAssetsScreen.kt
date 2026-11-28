package de.girocrab.app.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import de.girocrab.app.model.AppUiState
import de.girocrab.app.model.Holding
import de.girocrab.app.model.ManualLoan
import de.girocrab.app.model.TangibleAsset
import java.text.NumberFormat
import java.time.LocalDate
import java.util.Currency
import java.util.Locale
import kotlin.math.abs

private val GainGreen = Color(0xFF15803D)
private val LossRed = Color(0xFFB91C1C)
private const val MASK = "•••••"

private fun money(cents: Long, currency: String = "EUR", masked: Boolean = false): String {
    if (masked) return MASK
    val nf = NumberFormat.getCurrencyInstance(Locale.GERMANY)
    runCatching { nf.currency = Currency.getInstance(currency) }
    return nf.format(cents / 100.0)
}

private fun pct(value: Double, masked: Boolean): String =
    if (masked) MASK else String.format(Locale.GERMANY, "%+.2f %%", value)

private fun parseDecimal(s: String): Double? = s.trim().replace(".", "").replace(',', '.').toDoubleOrNull()

private fun parsePortfolioCents(s: String): Long? = parseDecimal(s)?.let { Math.round(it * 100) }

private fun qtyText(q: Double): String =
    NumberFormat.getNumberInstance(Locale.GERMANY).apply { maximumFractionDigits = 6 }.format(q)

private fun plainCents(c: Long): String = String.format(Locale.GERMANY, "%.2f", c / 100.0)

private fun parseDate(s: String): LocalDate? = runCatching { LocalDate.parse(s.trim()) }.getOrNull()

/** Depot, manual loans ("Kredite & Darlehen") and tangible assets ("Sachwerte"). */
@Composable
fun PortfolioAndAssetsScreen(
    state: AppUiState,
    onUpsertLoan: (ManualLoan) -> Unit,
    onDeleteLoan: (String) -> Unit,
    onUpsertAsset: (TangibleAsset) -> Unit,
    onDeleteAsset: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    var tab by rememberSaveable { mutableStateOf(0) }
    val titles = listOf("Depot", "Kredite & Darlehen", "Sachwerte")
    Column(modifier.fillMaxWidth()) {
        TabRow(selectedTabIndex = tab) {
            titles.forEachIndexed { i, t ->
                Tab(selected = tab == i, onClick = { tab = i }, text = { Text(t, maxLines = 1, overflow = TextOverflow.Ellipsis) })
            }
        }
        when (tab) {
            0 -> DepotTab(state)
            1 -> LoansTab(state, onUpsertLoan, onDeleteLoan)
            else -> AssetsTab(state, onUpsertAsset, onDeleteAsset)
        }
    }
}

// ---------------------------------------------------------------- Depot

@Composable
private fun DepotTab(state: AppUiState) {
    val m = state.privacyMode
    val holdings = state.data.holdings.filter { it.accountId in state.scopedAccountIds }
    val value = holdings.sumOf { it.valueCents }
    val cost = holdings.sumOf { it.costBasisCents }
    val gain = value - cost
    val gainPct = if (cost > 0) gain * 100.0 / cost else 0.0
    val gainColor = if (gain >= 0) GainGreen else LossRed

    LazyColumn(
        Modifier.fillMaxWidth(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Kpi("Depotwert", money(value, masked = m), Modifier.weight(1f))
                    Kpi("Einstand", money(cost, masked = m), Modifier.weight(1f))
                }
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Kpi("Gewinn/Verlust", money(gain, masked = m), Modifier.weight(1f), gainColor)
                    Kpi("Rendite", pct(gainPct, m), Modifier.weight(1f), gainColor)
                }
            }
        }
        if (holdings.isEmpty()) {
            item { Text("Keine Positionen.", color = MaterialTheme.colorScheme.onSurfaceVariant) }
        }
        items(holdings.sortedByDescending { it.valueCents }, key = { it.id }) { HoldingRow(it, m) }
    }
}

@Composable
private fun Kpi(title: String, value: String, modifier: Modifier = Modifier, valueColor: Color = Color.Unspecified) {
    Card(modifier, colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)) {
        Column(Modifier.padding(12.dp)) {
            Text(title, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(value, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold, color = valueColor, maxLines = 1)
        }
    }
}

@Composable
private fun HoldingRow(h: Holding, masked: Boolean) {
    val color = if (h.gainCents >= 0) GainGreen else LossRed
    val ids = listOf(h.isin, h.wkn).filter { it.isNotBlank() }.joinToString(" · ")
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                Text(h.name, Modifier.weight(1f), fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(money(h.valueCents, masked = masked), fontWeight = FontWeight.SemiBold, maxLines = 1)
            }
            if (ids.isNotEmpty()) {
                Text(ids, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Text(
                "${if (masked) MASK else qtyText(h.quantity)} Stk. · " +
                    // Priced in the account's currency; the purchase price only when the core has a
                    // native figure — otherwise the EUR amount would be dressed up as USD.
                    money(h.priceCents, h.quoteCurrency, masked) +
                        (h.nativePurchasePriceCents
                            ?.let { " (Kauf ${money(it, h.nativeCurrency, masked)})" }
                            ?: ""),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                "${money(h.gainCents, masked = masked)}  ${pct(h.gainPct, masked)}",
                style = MaterialTheme.typography.bodyMedium,
                color = color,
                fontWeight = FontWeight.Medium,
            )
        }
    }
}

// ---------------------------------------------------------------- Loans

@Composable
private fun LoansTab(state: AppUiState, onUpsert: (ManualLoan) -> Unit, onDelete: (String) -> Unit) {
    val m = state.privacyMode
    var editing by remember { mutableStateOf<ManualLoan?>(null) }
    var creating by remember { mutableStateOf(false) }
    val loans = state.data.loans
    ListWithAdd(
        total = "Restschuld gesamt: ${money(-loans.sumOf { it.remainingCents }, masked = m)}",
        empty = loans.isEmpty(),
        emptyText = "Keine Kredite oder Darlehen.",
        addLabel = "Kredit anlegen",
        onAdd = { if (state.demoActive) creating = true },
    ) {
        // The figures belong to the core: a Stand, its repayments and their split. Only the demo
        // keeps its own records; a real vault is edited on the desktop until Stufe 2 (#1430).
        if (!state.demoActive) {
            item {
                Text(
                    "Restschuld, Stichtag und Zinssatz pflegt der Desktop-Client. Bearbeiten auf Android " +
                        "kommt mit den Schreibpfaden (Stufe 2, #1430).",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        items(loans, key = { it.accountId }) { l ->
            val card = if (state.demoActive) Modifier.clickable { editing = l } else Modifier
            Card(Modifier.fillMaxWidth().then(card)) {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text(l.name, Modifier.weight(1f), fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(money(-l.remainingCents, masked = m), color = LossRed, fontWeight = FontWeight.SemiBold)
                    }
                    val repaid = l.repaid
                    if (repaid != null) {
                        androidx.compose.material3.LinearProgressIndicator(
                            progress = { repaid.toFloat() },
                            Modifier.fillMaxWidth().padding(vertical = 4.dp),
                        )
                        Text(
                            "${(repaid * 100).toInt()} % getilgt von ${money(l.originalCents!!, masked = m)}",
                            style = MaterialTheme.typography.bodySmall,
                        )
                    } else {
                        Text(
                            "Ursprünglicher Kreditbetrag fehlt im Tresor — kein Fortschritt rechenbar.",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Text(
                        loanPayments(l, m),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        loanSchedule(l) + (l.asOfDate?.let { " · Stand $it" } ?: ""),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
    if (state.demoActive && (creating || editing != null)) {
        LoanEditorDialog(
            initial = editing,
            onDismiss = { creating = false; editing = null },
            onSave = { onUpsert(it); creating = false; editing = null },
            onDelete = editing?.let { e -> { onDelete(e.accountId); editing = null } },
        )
    }
}

/** What the core reports about the repayments: how many, how much, and how it split. */
private fun loanPayments(l: ManualLoan, masked: Boolean): String =
    if (l.paymentCount == 0) {
        "Noch keine Rate verknüpft"
    } else {
        "Raten: ${l.paymentCount} · gezahlt ${money(l.paidCents, masked = masked)} " +
            "(Tilgung ${money(l.principalCents, masked = masked)}, Zins ${money(l.interestCents, masked = masked)})"
    }

/** Rate and remaining term; both come from the core, nothing is projected here. */
private fun loanSchedule(l: ManualLoan): String {
    val rate = l.interestRatePpm
        ?.let { String.format(Locale.GERMANY, "%.2f %% p. a.", it / 10_000.0) }
        ?: "zinsfrei"
    val rest = when (l.monthsLeft) {
        null -> "Restlaufzeit unbekannt"
        0 -> "getilgt"
        else -> "noch ${l.monthsLeft} Raten"
    }
    return "$rate · $rest"
}

/**
 * Demo editor. A real loan is a core account with a Stand, so writing one needs the core's
 * `createManualLoan`/`updateManualLoan` — Stufe 2 (#1430). Until then the fields here only fill
 * the demo snapshot, where no core knows better.
 */
@Composable
private fun LoanEditorDialog(
    initial: ManualLoan?,
    onDismiss: () -> Unit,
    onSave: (ManualLoan) -> Unit,
    onDelete: (() -> Unit)?,
) {
    var name by remember { mutableStateOf(initial?.name ?: "") }
    var principal by remember { mutableStateOf(initial?.originalCents?.let { plainCents(it) } ?: "") }
    var balance by remember { mutableStateOf(initial?.let { plainCents(it.anchorBalanceCents) } ?: "") }
    var rate by remember { mutableStateOf(initial?.interestRatePpm?.let { String.format(Locale.GERMANY, "%.2f", it / 10_000.0) } ?: "") }
    var stand by remember { mutableStateOf((initial?.anchorDate ?: LocalDate.now()).toString()) }

    val principalC = if (principal.isBlank()) null else parsePortfolioCents(principal)
    val balanceC = parsePortfolioCents(balance)
    val ratePct = if (rate.isBlank()) 0.0 else parseDecimal(rate)
    val standD = parseDate(stand)
    val error = when {
        name.isBlank() -> "Bitte einen Namen angeben."
        balanceC == null || balanceC < 0 -> "Restschuld ungültig."
        principal.isNotBlank() && (principalC == null || principalC <= 0) -> "Kreditbetrag ungültig."
        ratePct == null || ratePct < 0 || ratePct > 30 -> "Zinssatz muss zwischen 0 und 30 % liegen."
        standD == null -> "Datum im Format JJJJ-MM-TT."
        else -> null
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (initial == null) "Kredit anlegen" else "Kredit bearbeiten") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "Für Kredite ohne eigenes Konto bei der Bank, z. B. einen Autokredit. Die Restschuld gilt " +
                        "zum Stichtag; Raten danach zieht der Kern ab.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Field("Name", name, { name = it }, "z. B. Autokredit", KeyboardType.Text)
                Field("Ursprünglicher Kreditbetrag (€)", principal, { principal = it })
                Field("Restschuld am Stichtag (€)", balance, { balance = it })
                Field("Sollzins p. a. in %", rate, { rate = it }, "z. B. 4,99")
                Field("Stichtag (JJJJ-MM-TT)", stand, { stand = it }, keyboard = KeyboardType.Text)
                error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            }
        },
        confirmButton = {
            TextButton(
                enabled = error == null,
                onClick = {
                    val day = standD!!
                    onSave(
                        ManualLoan(
                            accountId = initial?.accountId ?: "l_${System.nanoTime()}",
                            name = name.trim(),
                            originalCents = principalC,
                            anchorDate = day,
                            anchorBalanceCents = balanceC!!,
                            interestRatePpm = ratePct!!.let { if (it > 0) Math.round(it * 10_000) else null },
                            matchExpr = "",
                            remainingCents = balanceC,
                            asOfDate = day,
                            paymentCount = 0,
                            paidCents = 0,
                            principalCents = 0,
                            interestCents = 0,
                            beforeAnchorCount = 0,
                            lastPaymentCents = null,
                            lastPaymentDate = null,
                            monthsLeft = null,
                        ),
                    )
                },
            ) { Text(if (initial == null) "Anlegen" else "Speichern") }
        },
        dismissButton = {
            Row {
                onDelete?.let { d ->
                    IconButton(onClick = d) { Icon(Icons.Filled.Delete, contentDescription = "Löschen", tint = MaterialTheme.colorScheme.error) }
                }
                TextButton(onClick = onDismiss) { Text("Abbrechen") }
            }
        },
    )
}

// ---------------------------------------------------------------- Assets

@Composable
private fun AssetsTab(state: AppUiState, onUpsert: (TangibleAsset) -> Unit, onDelete: (String) -> Unit) {
    val m = state.privacyMode
    var editing by remember { mutableStateOf<TangibleAsset?>(null) }
    var creating by remember { mutableStateOf(false) }
    val assets = state.data.assets
    ListWithAdd(
        total = "Sachwerte gesamt: ${money(assets.sumOf { it.valueCents }, masked = m)}",
        empty = assets.isEmpty(),
        emptyText = "Keine Sachwerte.",
        addLabel = "Sachwert anlegen",
        onAdd = { creating = true },
    ) {
        items(assets, key = { it.id }) { a ->
            Card(Modifier.fillMaxWidth().clickable { editing = a }) {
                Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Text(a.name, Modifier.weight(1f), fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(money(a.valueCents, masked = m), fontWeight = FontWeight.SemiBold)
                    }
                    Text(
                        "Erworben ${a.acquiredDate}" + if (a.note.isNotBlank()) " · ${a.note}" else "",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
    if (creating || editing != null) {
        AssetEditorDialog(
            initial = editing,
            onDismiss = { creating = false; editing = null },
            onSave = { onUpsert(it); creating = false; editing = null },
            onDelete = editing?.let { e -> { onDelete(e.id); editing = null } },
        )
    }
}

@Composable
private fun AssetEditorDialog(
    initial: TangibleAsset?,
    onDismiss: () -> Unit,
    onSave: (TangibleAsset) -> Unit,
    onDelete: (() -> Unit)?,
) {
    var name by remember { mutableStateOf(initial?.name ?: "") }
    var value by remember { mutableStateOf(initial?.let { plainCents(it.valueCents) } ?: "") }
    var date by remember { mutableStateOf((initial?.acquiredDate ?: LocalDate.now()).toString()) }
    var note by remember { mutableStateOf(initial?.note ?: "") }

    val valueC = parsePortfolioCents(value)
    val dateD = parseDate(date)
    val error = when {
        name.isBlank() -> "Bitte einen Namen angeben."
        valueC == null || valueC < 0 -> "Wert ungültig."
        dateD == null -> "Datum im Format JJJJ-MM-TT."
        else -> null
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(if (initial == null) "Sachwert anlegen" else "Sachwert bearbeiten") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Field("Name", name, { name = it }, "z. B. Fahrzeug, Immobilie", KeyboardType.Text)
                Field("Aktueller Wert (€)", value, { value = it })
                Field("Erwerbsdatum (JJJJ-MM-TT)", date, { date = it }, keyboard = KeyboardType.Text)
                Field("Notiz", note, { note = it }, keyboard = KeyboardType.Text)
                error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
            }
        },
        confirmButton = {
            TextButton(
                enabled = error == null,
                onClick = {
                    onSave(
                        TangibleAsset(
                            id = initial?.id ?: "a_${System.nanoTime()}",
                            name = name.trim(),
                            valueCents = valueC!!,
                            acquiredDate = dateD!!,
                            note = note.trim(),
                        ),
                    )
                },
            ) { Text(if (initial == null) "Anlegen" else "Speichern") }
        },
        dismissButton = {
            Row {
                onDelete?.let { d ->
                    IconButton(onClick = d) { Icon(Icons.Filled.Delete, contentDescription = "Löschen", tint = MaterialTheme.colorScheme.error) }
                }
                TextButton(onClick = onDismiss) { Text("Abbrechen") }
            }
        },
    )
}

// ---------------------------------------------------------------- Shared

@Composable
private fun ListWithAdd(
    total: String,
    empty: Boolean,
    emptyText: String,
    addLabel: String,
    onAdd: () -> Unit,
    content: androidx.compose.foundation.lazy.LazyListScope.() -> Unit,
) {
    LazyColumn(
        Modifier.fillMaxWidth(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
                Text(total, Modifier.weight(1f), style = MaterialTheme.typography.titleSmall)
                TextButton(onClick = onAdd) {
                    Icon(Icons.Filled.Add, contentDescription = null)
                    Text(addLabel, Modifier.padding(start = 4.dp))
                }
            }
        }
        if (empty) item { Text(emptyText, color = MaterialTheme.colorScheme.onSurfaceVariant) }
        content()
    }
}

@Composable
private fun Field(
    label: String,
    value: String,
    onChange: (String) -> Unit,
    placeholder: String? = null,
    keyboard: KeyboardType = KeyboardType.Decimal,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        placeholder = placeholder?.let { { Text(it) } },
        singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = keyboard),
        modifier = Modifier.fillMaxWidth(),
    )
}
