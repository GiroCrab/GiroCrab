package de.girocrab.app.model

/**
 * Reads a booking list to the end instead of stopping after the first page (#1429, 3.3).
 *
 * Until now the app read exactly one page of 1.000 rows and said so in the list. That is visible
 * for the list, but not for the analytics screen: it sums whatever arrived, so a vault with more
 * bookings quietly produced smaller totals. Reading all pages at unlock costs one query per 1.000
 * rows and removes that wrong number; how *many* rows a page holds stays with the caller.
 *
 * The loop assumes every page is filled from the offset that was asked for — true for
 * [VaultSnapshotMapper.firstPageFilter], which sets `wholeDays = false`. With `wholeDays = true`
 * the core may extend a page past its limit to complete the last day, and the next offset would
 * skip rows; callers that need that must not use this helper.
 *
 * The page cap is a guard, not a feature: a vault that needs more than [MAX_PAGES] pages reports
 * the rest as missing (`truncated`) instead of blocking the unlock for seconds.
 */
object BookingsPaging {

    /** Rows per query. Same value the first page always used. */
    const val PAGE_SIZE = 1_000

    /** Upper bound of queries per list; 100 pages = 100.000 bookings. */
    const val MAX_PAGES = 100

    data class Page<T>(val rows: List<T>, val totalCount: Int)

    data class Coverage<T>(val rows: List<T>, val totalCount: Int) {
        /** True when the cap ended the read before the core's own count was reached. */
        val truncated: Boolean get() = rows.size < totalCount
    }

    /**
     * Fetches pages from `offset = 0` upwards until the core's `totalCount` is covered, a page
     * comes back empty, or [maxPages] is spent. An empty page always ends the read, even when
     * `totalCount` claims more: looping on a counter that disagrees with the data never ends.
     */
    fun <T> readAll(
        pageSize: Int = PAGE_SIZE,
        maxPages: Int = MAX_PAGES,
        fetch: (offset: Int, limit: Int) -> Page<T>,
    ): Coverage<T> {
        require(pageSize > 0) { "pageSize must be positive, was $pageSize" }
        require(maxPages > 0) { "maxPages must be positive, was $maxPages" }
        val rows = ArrayList<T>()
        var total = 0
        var pages = 0
        while (pages < maxPages) {
            val page = fetch(rows.size, pageSize)
            total = page.totalCount
            if (page.rows.isEmpty()) break
            rows += page.rows
            pages++
            if (rows.size >= total) break
        }
        // A core that reports fewer rows than it just returned must not produce "0 of -3".
        return Coverage(rows, maxOf(total, rows.size))
    }
}
