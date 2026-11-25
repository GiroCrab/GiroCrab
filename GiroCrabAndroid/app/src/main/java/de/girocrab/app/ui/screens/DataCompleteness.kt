package de.girocrab.app.ui.screens

/**
 * Note for screens whose totals depend on the booking rows the app holds.
 *
 * Analytics sums `state.data.bookings`. Since #1429 (3.3) every page is read at unlock, but
 * [de.girocrab.app.model.BookingsPaging.MAX_PAGES] is a hard cap: above 100.000 rows the rest
 * stays unread and `bookingsTotal` is larger than the loaded list. The booking list says so; the
 * numbers on the analytics screen must carry the same statement, otherwise month sums, category
 * shares and budgets look complete while they are quietly too small.
 */
fun incompleteBookingsNote(loaded: Int, total: Int): String? =
    if (total > loaded) {
        "Daten unvollständig: $loaded von $total Buchungen geladen. " +
            "Monatssummen, Kategorienanteile und Budgets beziehen sich nur auf die geladenen Buchungen."
    } else {
        null
    }
