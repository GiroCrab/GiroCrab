import Foundation
import SwiftUI
import GiroCrabCore

extension AppModel {
    // MARK: - Sync (two-phase)

    /// Starts a sync (or the login it needs) once the banking policy is fresh
    /// and allows this login; otherwise explains why not.
    func requestSync(_ connection: BankConnection) {
        // iOS: the main view presents what a sync asks or reports (PIN, TAN, setup form,
        // browser login, error), and none of it can appear over the Einstellungen sheet
        // (#1831). Close the sheet; the sync starts once it is gone.
        if showSettings {
            let id = connection.id
            let current = vault
            // What this request asked for besides the bookings (statements, Postbox, one
            // account): another login's sync while the sheet closes would clear it.
            let statements = statementsAccountID
            let statementsBefore = statementsCountBefore
            let postbox = postboxConnectionID
            let accountOnly = syncAccountID
            afterSettingsDismiss = { [weak self] in
                // locked or another vault meanwhile: the id may name another login there
                guard let self, let current, self.vault === current,
                      let c = self.connections.first(where: { $0.id == id }) else { return }
                // startSync still drops a target that is not of exactly this login
                self.statementsAccountID = statements
                self.statementsCountBefore = statementsBefore
                self.postboxConnectionID = postbox
                self.syncAccountID = accountOnly
                self.requestSync(c)
            }
            showSettings = false
            return
        }
        // A login of a run waits here for the policy check: "Alle abbrechen", a lock or a
        // closed window meanwhile must not start it afterwards (#2055 review).
        let runGeneration = syncRunActive && syncRun?.contains(connection.id) == true ? syncGeneration : nil
        withFreshBankingPolicy { [self] in
            if let runGeneration {
                guard syncGeneration == runGeneration,
                      let status = syncRun?.status(of: connection.id), status == .waiting || status == .running
                else { return }
            }
            if let gate = bankingGate(for: connection) {
                clearSyncTargets()
                if syncRunActive {
                    // A lock published while "Alle abrufen" runs: skip this login and go on
                    // with the rest; a lock of all logins skips them all. The window says why.
                    if gate.all {
                        syncQueue = []
                        syncRun?.skipRemaining(reason: String(localized: "Übersprungen: \(gate.title)"))
                        syncNextInQueue()   // the run is over: announce its summary
                    } else {
                        syncRun?.skip(connection.id, reason: Self.skippedByLock)
                        syncNextInQueue()
                    }
                } else {
                    resetSyncBatch()
                    showError(gate.title, gate.detail)
                }
                return
            }
            requestSyncChecked(connection)
        }
    }

    /// The Einstellungen sheet is gone (its `onDismiss`): starts the sync that waited for it.
    func settingsDismissed() {
        let next = afterSettingsDismiss
        afterSettingsDismiss = nil
        next?()
    }

    private func requestSyncChecked(_ connection: BankConnection) {
        syncRun?.start(connection.id)
        // a new attempt: an import an earlier browser window reported does not count for it
        syncRunBrowserImports[connection.id] = nil
        // Bitcoin Core on a Mac without a node (the vault came over iCloud): not fetched here,
        // its balance comes from the Mac running the node
        if connection.connector == .bitcoinCore, !isDemo,
           !BitcoinCoreDatadir.nodeOnThisMac(configured: settingEntry(BitcoinCoreDatadir.key)?.value) {
            if syncRunActive {
                syncRun?.skip(connection.id, reason: String(localized: "Übersprungen: wird per iCloud-Sync vom Mac mit Bitcoin Core aktualisiert"))
            } else {
                toast = String(localized: "\(connection.displayName): wird per iCloud-Sync vom Mac mit Bitcoin Core aktualisiert.")
            }
            syncNextInQueue()
            return
        }
        // Bitcoin Core: the data directory Bitcoin-Qt uses, unless one is set (stored, so it
        // shows in the connection's settings and can be changed there)
        if connection.connector == .bitcoinCore, !isDemo, settingEntry(BitcoinCoreDatadir.key)?.hasValue != true,
           let dir = BitcoinCoreDatadir.detected() {
            saveSetting(BitcoinCoreDatadir.key, dir)
        }
        if connection.connector == .advanzia, !isDemo, !Self.isMagicDemoLogin(connection.loginName) {
            // Its own window and flow, which cannot run inside the sync window.
            if syncRunActive {
                syncRun?.skip(connection.id, reason: String(localized: "Übersprungen: eigener Abruf-Ablauf, bitte einzeln abrufen"))
                syncNextInQueue()
                return
            }
            startAdvanzia(connection); return
        }
        // Finpension: no sync, the export file is imported from the connection's sheet.
        if usesFileImport(connection) {
            if syncRunActive {
                syncRun?.skip(connection.id, reason: String(localized: "Übersprungen: wird per Datei-Import aktualisiert"))
                syncNextInQueue()
                return
            }
            resetSyncBatch()
            toast = String(localized: "\(connection.displayName): Bitte den Export in den Einstellungen der Verbindung importieren.")
            return
        }
        // Amex / Amazon Visa: embedded web-portal login (no API keys needed).
        if usesPortal(connection) {
            webLoginConnection = connection
            return
        }
        // Trade Republic: AWS WAF blocks raw logins – without a stored
        // session the in-app browser login is the default flow.
        let magicDemo = Self.isMagicDemoLogin(connection.loginName)
        if !isDemo, !magicDemo, connection.connector == .tradeRepublic, !(connectionStatus(connection.id)?.hasSession ?? false) {
            webLoginConnection = connection
            return
        }
        if !isDemo, let st = connectionStatus(connection.id), (st.hasPin || ([.tradeRepublic, .kontist].contains(connection.connector) && st.hasSession)),
           missingSettings(for: connection).isEmpty {
            startSync(connection, secret: "")
            return
        }
        if !isDemo {
            let missing = missingSettings(for: connection)
            if !missing.isEmpty {
                setupRequest = SetupRequest(connection: connection, keys: missing)
                return
            }
        }
        let needsSecret = !(bankCatalog().first { $0.kind == connection.connector }?.secretLabel.hasPrefix("(") ?? false)
        if isDemo || !needsSecret {
            startSync(connection, secret: isDemo ? "demo" : "")
        } else {
            pendingLogin = PendingLogin(connection: connection)
        }
    }

    /// "Abrufen" of a login: all its accounts, also after an earlier
    /// "<Konto> abrufen" of it ended in an error.
    func requestFullSync(_ connection: BankConnection) {
        syncAccountID = nil
        requestSync(connection)
    }

    /// "<Konto> abrufen": only this account of its login (#1124). Login
    /// prompts and retries of this sync keep to it.
    func requestSyncAccount(_ account: Account) {
        guard syncingConnectionID == nil, !account.isArchived, let cid = account.connectionId,
              let connection = connections.first(where: { $0.id == cid }) else { return }
        if supportsAccountSync(connection) { syncAccountID = account.id }
        requestSync(connection)
    }

    /// Logins with their own flow (web portal, file import, Advanzia) fetch
    /// everything, also for "<Konto> abrufen".
    func supportsAccountSync(_ connection: BankConnection) -> Bool {
        !usesPortal(connection) && !usesFileImport(connection) && connection.connector != .advanzia
    }

    /// Whether the sidebar shows `account` as being fetched: its login syncs,
    /// and the sync is not limited to another of its accounts (#1572).
    func isSyncing(_ account: Account) -> Bool {
        guard let cid = syncingConnectionID, cid == account.connectionId else { return false }
        return syncingAccountID == nil || syncingAccountID == account.id
    }

    func requestSyncAll() {
        requestSyncBatch { [self] in
            connections.filter {
                !$0.allAccountsArchived && supportsSyncSchedule($0) && syncSchedule($0.id) != AutoSync.never
            }
        }
    }

    /// "Konten abrufen" in a group's menu: the logins behind the group's
    /// accounts, subgroups included, each once – as "Alle abrufen" would.
    func groupSyncConnections(_ group: UserGroup) -> [BankConnection] {
        let ids = Set(userGroupSubtree(group.uuid).flatMap(\.accounts)
            .filter { !$0.isArchived }.compactMap(\.connectionId))
        return connections.filter { ids.contains($0.id) && !usesPortal($0) && !usesFileImport($0) }
    }

    func requestSyncGroup(_ group: UserGroup) {
        requestSyncBatch { [self] in groupSyncConnections(group) }
    }

    /// What a pull-to-refresh starts besides the iCloud merge.
    enum RefreshSyncAction: Equatable {
        case none
        case account(Int64)
        case group(String)
        case all
    }

    /// Context-aware bank sync of the current sidebar selection. Import-only
    /// and portal accounts (interactive login) need no bank sync: the iCloud
    /// merge and reload cover them.
    func refreshSyncAction() -> RefreshSyncAction {
        switch sidebarSelection {
        case .account(let id):
            guard let account = accountsByID[id], !account.isArchived,
                  let c = connections.first(where: { $0.id == account.connectionId }),
                  !usesPortal(c), !usesFileImport(c) else { return .none }
            return .account(id)
        case .userGroup(let uuid) where !uuid.isEmpty:
            return .group(uuid)
        case .documents, .wealth:
            return .none
        default:
            return .all
        }
    }

    /// Pull-to-refresh: merges the iCloud snapshots first (bookings synced on
    /// the Mac), then syncs what the current selection shows. Returns when the
    /// work is done, so the native spinner stays up. While a sync runs it only
    /// waits for it instead of starting a second one.
    func refreshCurrentView() async {
        if let v = ownVault, CloudSyncManager.shared.isEnabled {
            await CloudSyncManager.shared.sync(vault: v)
            reloadAll()
        }
        guard syncingConnectionID == nil else {
            await awaitSyncSettled()
            return
        }
        let action = refreshSyncAction()
        guard action != .none else { return }
        if let hook = refreshSyncOverride { hook(action); return }
        switch action {
        case .none: return
        case .account(let id):
            guard let account = accountsByID[id] else { return }
            requestSyncAccount(account)
        case .group(let uuid):
            guard let g = userGroups.first(where: { $0.uuid == uuid }) else { return }
            requestSyncGroup(g)
        case .all:
            requestSyncAll()
        }
        await awaitSyncSettled()
    }

    /// Waits until the running sync (batch included) is over, or stops at a
    /// PIN / TAN / setup prompt, which only the user can answer.
    private func awaitSyncSettled() async {
        var waited = 0
        // the banking policy check may delay the start a moment
        try? await Task.sleep(for: .milliseconds(150))
        while !Task.isCancelled, waited < 600 {
            // an error alert of "Alle abrufen" waits for the user too (#1041)
            let prompting = pendingLogin != nil || setupRequest != nil || webLoginConnection != nil || activeChallenge != nil
                || errorMessage != nil
            // the sync window shows the rest of "Alle abrufen" (#1041)
            if prompting || syncRun != nil || (syncingConnectionID == nil && syncQueue.isEmpty) { return }
            try? await Task.sleep(for: .milliseconds(200))
            waited += 1
        }
    }

    /// Runs `logins` one after another; read once the banking policy is fresh.
    private func requestSyncBatch(_ logins: @escaping @MainActor () -> [BankConnection]) {
        guard syncingConnectionID == nil, syncRun == nil else { return }
        // iOS: the sync window cannot appear over the Einstellungen sheet (#1831)
        if showSettings {
            afterSettingsDismiss = { [weak self] in self?.requestSyncBatch(logins) }
            showSettings = false
            return
        }
        refreshAssetPrices()
        withFreshBankingPolicy { [self] in
            guard syncingConnectionID == nil, syncRun == nil else { return }
            let batch = logins()
            // Logins the banking policy blocks are skipped; the window names them.
            let blocked = batch.filter { bankingGate(for: $0) != nil }
            let allowed = batch.filter { bankingGate(for: $0) == nil }
            guard let head = allowed.first else {
                if let gate = blocked.first.flatMap({ bankingGate(for: $0) }) {
                    showError(gate.title, gate.detail)
                }
                return
            }
            syncQueue = Array(allowed.dropFirst())
            syncAccountID = nil
            syncRunActions = [:]
            if batch.count > 1 {
                var run = SyncRun(logins: batch.map { ($0.id, $0.displayName) })
                for b in blocked { run.skip(b.id, reason: Self.skippedByLock) }
                syncRun = run
            }
            requestSyncChecked(head)
        }
    }

    /// Position in a running "Alle abrufen" over several logins, e.g. (2, 5).
    var syncBatchProgress: (current: Int, total: Int)? {
        guard let run = syncRun, !run.isFinished else { return nil }
        return (run.position, run.total)
    }

    /// A run over several logins is under way (its window is open and not yet over).
    var syncRunActive: Bool { syncRun.map { !$0.isFinished } ?? false }

    /// In a run, cancelling a PIN or TAN prompt skips this login (the window
    /// keeps it as "übersprungen") instead of stopping the whole run.
    var canSkipInSyncBatch: Bool { syncRunActive }

    /// Cancel button of the TAN sheet. In a batch it drops this login and goes
    /// on with the next one (on the last one it ends the batch with its notice).
    func skipCurrentSync() {
        guard syncRunActive, activeTransfer == nil else { cancelSync(); return }
        if let id = activeChallenge?.connection.id ?? syncingConnectionID {
            syncRun?.skip(id, reason: Self.skippedByUser)
            syncRunBrowserImports[id] = nil
        }
        pollTask?.cancel()
        let s = activeSession
        activeSession = nil
        activeChallenge = nil
        clearSyncTargets()
        syncGeneration &+= 1
        let generation = syncGeneration
        Task {
            // end the bank dialog first: the next login may be the same bank.
            // syncingConnectionID stays set until then, so neither a scheduled
            // sync nor a new "Alle abrufen" can start in between.
            await s?.cancel()
            // "Alle abbrechen", a lock or a new sync meanwhile own the state now:
            // never end a session this skip did not start.
            guard self.syncGeneration == generation else { return }
            self.syncingConnectionID = nil
            self.syncNextInQueue()
        }
    }

    /// Reason in the sync window for a login the user skipped.
    static var skippedByUser: String { String(localized: "Von Ihnen übersprungen") }
    /// Reason for a login the banking policy blocks (security lock).
    static var skippedByLock: String { String(localized: "Wegen Sicherheitssperre übersprungen") }

    /// "Alle abbrechen": ends the running sync and closes whichever prompt of
    /// the batch is open (PIN, setup, browser login, TAN). The sync window stays
    /// with every login not yet fetched as skipped.
    func cancelAllSync() {
        advanziaWorkflow?.cancel()
        setupRequest = nil
        webLoginConnection = nil   // in a batch only the Trade Republic login
        cancelSync()
    }

    /// "Abbrechen" / "Überspringen" in the PIN sheet.
    func cancelPendingLogin() {
        let c = pendingLogin?.connection
        pendingLogin = nil
        skipPrompt(for: c)
    }

    /// "Abbrechen" / "Überspringen" in the setup sheet (missing settings).
    func cancelSetupRequest() {
        let c = setupRequest?.connection
        setupRequest = nil
        skipPrompt(for: c)
    }

    /// "Abbrechen" / "Überspringen" in the Trade Republic browser login.
    func cancelWebLogin() {
        let c = webLoginConnection
        webLoginConnection = nil
        skipPrompt(for: c)
    }

    /// A prompt before the bank dialog (PIN, setup, browser login) was
    /// cancelled: in a batch go on with the next login, otherwise end it –
    /// a stale queue would otherwise resume after a later single sync.
    private func skipPrompt(for connection: BankConnection?) {
        // A cancelled "Kontoauszüge abrufen" must never carry over to the next
        // sync – it would send that login's PIN for another bank's account.
        clearSyncTargets()
        guard syncRunActive else { resetSyncBatch(); return }
        if let id = connection?.id {
            syncRun?.skip(id, reason: Self.skippedByUser)
            syncRunBrowserImports[id] = nil
        }
        syncNextInQueue()
    }

    /// Ends a run at once, its window included (lock, off to Settings).
    func resetSyncBatch() {
        syncQueue = []
        syncRun = nil
        syncRunActions = [:]
        syncRunBrowserImports = [:]
    }

    /// "Fertig" in the sync window once the run is over.
    func closeSyncRun() {
        guard let run = syncRun, run.isFinished else { return }
        syncRun = nil
        syncRunActions = [:]
        syncRunBrowserImports = [:]
        // held back while the window was open: its sheet cannot show over it (#346)
        checkPendingAccountSelection()
    }

    /// "Erneut versuchen" of a failed or skipped login in the sync window: it is fetched
    /// again after the logins still waiting, or right away when the run is over.
    func retryInSyncRun(_ id: Int64) {
        guard var run = syncRun, let c = connections.first(where: { $0.id == id }) else { return }
        let idle = run.isFinished && syncingConnectionID == nil
        guard run.retry(id) else { return }
        syncRun = run
        syncRunActions[id] = nil
        syncRunBrowserImports[id] = nil
        syncQueue.append(c)
        if idle { syncNextInQueue() }
    }

    /// An extra action of a failed login (another FinTS registration …): closes the
    /// window, then runs it as a single sync would.
    func runSyncRunAction(_ id: Int64, _ index: Int) {
        guard let actions = syncRunActions[id], actions.indices.contains(index) else { return }
        let action = actions[index].action
        closeSyncRun()
        action()
    }

    /// The browser window of a run's login imported it itself (FNZ): its row is done.
    /// Counted when the window closes, which moves the run on.
    func webLoginImportedInSyncRun(_ id: Int64, imported: Int) {
        guard syncRun?.status(of: id) == .running else { return }
        syncRunBrowserImports[id] = imported
    }

    /// A browser login of the run closed without a sync of its own: the run goes on. Only an
    /// import the window reported (`webLoginImportedInSyncRun`) counts as fetched; any other
    /// close leaves the login out.
    func webLoginClosedInSyncRun(_ connection: BankConnection) {
        guard syncRunActive else { return }
        let generation = syncGeneration
        Task { @MainActor [weak self] in
            // a skip, a sync started from the browser session or a cancel acts first
            guard let self, self.syncGeneration == generation, self.syncingConnectionID == nil,
                  self.syncRun?.running?.connectionID == connection.id,
                  self.pendingLogin == nil, self.setupRequest == nil, self.activeChallenge == nil,
                  self.webLoginConnection == nil else { return }
            if let imported = self.syncRunBrowserImports.removeValue(forKey: connection.id) {
                self.syncRun?.finish(connection.id, imported: imported,
                                     detail: String(localized: "Im Browser-Fenster abgerufen"))
            } else {
                self.syncRun?.skip(connection.id, reason: Self.browserClosed)
            }
            self.syncNextInQueue()
        }
    }

    /// Errors answered with a prompt (setup form, browser login, FinTS registration)
    /// instead of a failure: in "Alle abrufen" the batch waits for the prompt, whose
    /// "Überspringen" goes on with the next login (#1041). The Trade Republic PIN
    /// prompt (#1128) depends on the login, so `finishSync` handles it before this.
    nonisolated static func errorOpensPrompt(_ error: Error) -> Bool {
        if isMissingProductID(error) { return true }
        guard let ce = error as? CoreError else { return false }
        switch ce {
        case .BrowserLoginRequired, .MissingSetting: return true
        default: return false
        }
    }

    /// Toast suffix of a sync with the comdirect Postbox (#838) or the
    /// Consorsbank Online-Archiv (#1024).
    static func postboxNote(_ s: SyncSummary, _ c: BankConnection) -> String {
        if c.connector == .finTsGeneric, c.bankCode == "76030080" {
            guard let n = s.postboxDocuments else {
                return String(localized: "Online-Archiv nicht abgerufen. Bitte PIN und Freigabe in der Consorsbank App prüfen.")
            }
            return n == 0
                ? String(localized: "keine neuen Dokumente aus dem Online-Archiv")
                : String(localized: "\(n) neue Dokumente aus dem Online-Archiv")
        }
        guard let n = s.postboxDocuments else {
            return String(localized: "Postbox nicht abgerufen. Bitte Client-ID und Client-Secret der comdirect-API prüfen.")
        }
        return n == 0
            ? String(localized: "keine neuen Postbox-Dokumente")
            : String(localized: "\(n) neue Postbox-Dokumente")
    }

    /// Shown after a sync that left history for the next one.
    static func historyNote(_ s: SyncSummary) -> String {
        guard s.historyIncomplete else { return "" }
        let progress = HistoryProgress(remaining: Int(s.historyRemaining), total: Int(s.historyTotal)).text
        return " · " + String(localized: "Historie noch nicht vollständig – bitte noch einmal abrufen")
            + (progress.isEmpty ? "" : " (\(progress))")
    }

    /// Toast suffix for bookings the bank delivered for archived accounts.
    static func skippedArchivedNote(_ s: SyncSummary) -> String {
        s.transactionsSkippedArchived == 0 ? "" : " · " + String(localized: "\(s.transactionsSkippedArchived) Buchungen archivierter Konten übersprungen")
    }

    private func syncNextInQueue() {
        guard syncQueue.isEmpty else {
            requestSync(syncQueue.removeFirst())
            return
        }
        // run over: the window stays with its summary until "Fertig"
        if let run = syncRun, run.isFinished {
            AccessibilityNotification.Announcement(Self.syncRunSummary(run)).post()
        }
    }

    /// "Abruf abgeschlossen: 4 abgerufen, 1 übersprungen, 0 fehlgeschlagen".
    static func syncRunSummary(_ run: SyncRun) -> String {
        String(localized: "Abruf abgeschlossen: \(run.doneCount) abgerufen, \(run.skippedCount) übersprungen, \(run.failedCount) fehlgeschlagen")
    }

    /// Forgets what the next sync was asked to fetch besides (or instead of)
    /// the login's bookings: statements, Postbox, one account.
    func clearSyncTargets() {
        statementsAccountID = nil
        postboxConnectionID = nil
        syncAccountID = nil
    }

    /// `accountID` when it is an account of exactly this login.
    private func account(_ accountID: Int64?, of connection: BankConnection) -> Int64? {
        accountID.flatMap { accountsByID[$0]?.connectionId == connection.id ? $0 : nil }
    }

    /// Like `account(_:of:)`, and never in a scheduled sync.
    private func ownAccount(_ accountID: Int64?, of connection: BankConnection) -> Int64? {
        quietSyncConnectionID == connection.id ? nil : account(accountID, of: connection)
    }

    func startSync(_ connection: BankConnection, secret: String) {
        guard let vault else { return }
        if connection.connector == .advanzia, !isDemo, !Self.isMagicDemoLogin(connection.loginName) {
            startAdvanzia(connection); return
        }
        if usesPortal(connection) { webLoginConnection = connection; return }
        pendingLogin = nil
        // Statements only for an account of exactly this login: a stale value
        // would hand this login's PIN to another bank.
        if ownAccount(statementsAccountID, of: connection) == nil { statementsAccountID = nil }
        let statementsFor = statementsAccountID
        // "Postbox jetzt abrufen" only for exactly this login, never in an automatic sync
        if postboxConnectionID != connection.id || quietSyncConnectionID == connection.id {
            postboxConnectionID = nil
        }
        let postboxNow = postboxConnectionID != nil
        // kept when another login syncs first (e.g. a scheduled one)
        let accountOnly = ownAccount(syncAccountID, of: connection)
        let quiet = quietSyncConnectionID == connection.id
        syncingConnectionID = connection.id
        syncingAccountID = statementsFor ?? accountOnly
        syncVaultRevision = vaultRevision
        syncGeneration &+= 1
        Task {
            var started: SyncSession?
            do {
                let session: SyncSession
                if let accountID = statementsFor {
                    session = try vault.fetchAccountStatements(accountId: accountID, secret: secret, demo: isDemo)
                } else if postboxNow {
                    session = try vault.fetchComdirectPostbox(connectionId: connection.id, secret: secret, demo: isDemo)
                } else if let accountID = accountOnly {
                    session = try vault.beginAccountSync(accountId: accountID, secret: secret, demo: isDemo)
                } else if quiet {
                    // no new approval in the background (e.g. the comdirect Postbox, #838)
                    session = try vault.beginQuietSync(connectionId: connection.id, secret: secret, demo: isDemo)
                } else {
                    session = try vault.beginSync(connectionId: connection.id, secret: secret, demo: isDemo)
                }
                started = session
                self.activeSession = session
                let step = try await session.start()
                guard self.activeSession === session else { return }  // skipped / cancelled meanwhile
                // a TAN or skip answered meanwhile owns the sheet, unless the sync ended with .finished
                if self.activeChallenge?.busy == true {
                    guard case .finished = step else { return }
                }
                self.handle(step, connection: connection)
            } catch {
                if let started, self.activeSession !== started { return }
                // Kept for the "use GiroCrab's registration" retry of a rejected product ID,
                // so the login is not asked again; finishSync takes it and clears it.
                if statementsFor == nil { self.failedSyncSecret = secret }
                self.finishSync(error: error)
            }
        }
    }

    func handle(_ step: SyncStep, connection: BankConnection) {
        // A scheduled sync never asks for a login: the bank wants a new
        // consent, so stop quietly – the connection view shows it.
        if quietSyncConnectionID == connection.id {
            switch step {
            case .needsChallenge, .stillWaiting:
                logSync(connectionID: connection.id, source: .automatic, outcome: .cancelled,
                        message: String(localized: "Automatischer Abruf übersprungen: Die Bank verlangt eine neue Freigabe."))
                quietSyncConnectionID = nil
                let s = activeSession
                activeSession = nil
                syncingConnectionID = nil
                Task { await s?.cancel() }
                checkConsentExpiry()
                return
            case .finished:
                break
            }
        }
        switch step {
        case .needsChallenge(let c):
            activeChallenge = ActiveChallenge(connection: connection, challenge: c, waiting: false,
                                              canSkipOrder: activeSession?.isFetching() ?? false)
            if c.kind == .decoupledPush || c.kind == .progress { schedulePoll(connection: connection, interval: c.pollIntervalSeconds) }
        case .stillWaiting(let c):
            if activeChallenge == nil {
                activeChallenge = ActiveChallenge(connection: connection, challenge: c, waiting: true,
                                                  canSkipOrder: activeSession?.isFetching() ?? false)
            } else {
                activeChallenge?.challenge = c  // progress steps bring a new text
                activeChallenge?.canSkipOrder = activeSession?.isFetching() ?? false
                activeChallenge?.polling = false
                activeChallenge?.waiting = true
                activeChallenge?.busy = false
            }
            schedulePoll(connection: connection, interval: c.pollIntervalSeconds)
        case .finished(let s):
            // a scheduled sync of another login leaves a pending "<Konto> abrufen" alone
            if account(syncAccountID, of: connection) != nil { syncAccountID = nil }
            // what the bank would not give is no success (#1627)
            logSync(connectionID: connection.id, source: syncLogSource(for: connection.id),
                    outcome: s.warnings.isEmpty ? .ok : .partial,
                    message: connection.connector == .tradeRepublic ? Self.syncLogMessage(s) : (s.warnings + s.hints).joined(separator: " / "),
                    imported: Int(s.transactionsImported))
            activeChallenge = nil
            syncingConnectionID = nil
            activeSession = nil
            // Only a sync of the whole login in this vault tells the history's state: one account
            // ("<Konto> abrufen", statements) leaves the history and its documents for later.
            if syncingAccountID == nil, syncVaultRevision == vaultRevision {
                historyIncomplete[connection.id] = s.historyIncomplete
                    ? HistoryProgress(remaining: Int(s.historyRemaining), total: Int(s.historyTotal)) : nil
            }
            let statementsFetch = statementsAccountID != nil
            if let accountID = statementsAccountID {
                statementsAccountID = nil
                let before = statementsCountBefore
                let name = connection.displayName
                let warnings = s.warnings
                let report = { [self] (unplaced: Int) in
                    documentsVersion += 1
                    let n = max(0, accountDocuments(accountID).count - before)
                    var note = n > 0
                        ? String(localized: "\(name): \(n) neue Kontoauszüge")
                        : String(localized: "\(name): keine neuen Kontoauszüge. Falls die Bank keine elektronischen Kontoauszüge per FinTS anbietet, lassen sie sich hier nicht abrufen.")
                    // a partial download or a failed placement must not look like a finished fetch
                    if !warnings.isEmpty { note += " · " + warnings.joined(separator: " / ") }
                    if unplaced > 0 { note += " · " + String(localized: "\(unplaced) Dokumente konnten keinem Konto zugeordnet werden.") }
                    toast = note
                }
                if connection.connector == .bhw {
                    // BHW: mailbox PDFs reach their account only once their text was read.
                    Task { @MainActor in
                        report(await self.placeBhwDocuments().failed)
                    }
                } else {
                    report(0)
                }
            } else if postboxConnectionID == connection.id {
                postboxConnectionID = nil
                documentsVersion += 1
                toast = syncNotice(connection, s) + " · " + Self.postboxNote(s, connection)
            } else {
                let quiet = quietSyncConnectionID == connection.id
                if !quiet, syncRun?.status(of: connection.id) == .running {
                    // the window's row tells it instead of a notice
                    var parts = syncNoticeDetails(s)
                    // both notes start with " · " for the toast
                    for note in [Self.skippedArchivedNote(s), Self.historyNote(s)] where !note.isEmpty {
                        parts.append(String(note.dropFirst(" · ".count)))
                    }
                    if let n = s.postboxDocuments, n > 0 { parts.append(Self.postboxNote(s, connection)) }
                    syncRun?.finish(connection.id, imported: Int(s.transactionsImported),
                                    detail: parts.isEmpty ? nil : parts.joined(separator: " · "))
                } else if !quiet || s.transactionsImported > 0 || !s.newAccounts.isEmpty || !s.revokedAccounts.isEmpty
                    || !s.warnings.isEmpty {
                    var note = syncNotice(connection, s) + Self.skippedArchivedNote(s) + Self.historyNote(s)
                    if let n = s.postboxDocuments, n > 0 { note += " · " + Self.postboxNote(s, connection) }
                    toast = note
                }
                if s.postboxDocuments.map({ $0 > 0 }) ?? false { documentsVersion += 1 }
            }
            let wasQuiet = quietSyncConnectionID == connection.id
            quietSyncConnectionID = nil
            if wasQuiet { reloadAllInBackground() } else { reloadAll() }
            // A regular BHW sync can bring mailbox PDFs too (automatic statements).
            if connection.connector == .bhw && !statementsFetch {
                Task { @MainActor in
                    if await self.placeBhwDocuments().placed > 0 { self.documentsVersion += 1 }
                }
            }
            // its sheet cannot show over the sync window: asked once that is closed
            if syncRun == nil { checkPendingAccountSelection() }
            syncNextInQueue()
        }
    }

    func schedulePoll(connection: BankConnection, interval: UInt32) {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(max(interval, 1)) * 1_000_000_000)
            guard !Task.isCancelled, let self else { return }
            if let transfer = self.activeTransfer {
                do {
                    let step = try await transfer.poll()
                    guard self.activeTransfer === transfer else { return }  // cancelled meanwhile
                    self.handleTransfer(step, connection: connection)
                } catch {
                    guard self.activeTransfer === transfer else { return }
                    self.finishTransfer(error: error)
                }
                return
            }
            guard let session = self.activeSession else { return }
            if self.activeChallenge?.canSkipOrder == true { self.activeChallenge?.polling = true }
            do {
                let step = try await session.poll()
                guard self.activeSession === session else { return }  // skipped / cancelled meanwhile
                if self.activeChallenge?.busy == true {
                    // a TAN or skip answered meanwhile owns the sheet, unless the sync ended with .finished
                    guard case .finished = step else { return }
                }
                self.handle(step, connection: connection)
            } catch {
                guard self.activeSession === session else { return }
                self.finishSync(error: error)
            }
        }
    }

    /// Consorsbank Online-Archiv (#1024): the approval dialog's link switches between the
    /// app approval and a typed TAN ("TAN" / "APP", handled in `consorsbank_archive.rs`).
    func switchApproval(_ word: String) {
        pollTask?.cancel()
        submitCode(word)
    }

    func submitCode(_ code: String) {
        if let transfer = activeTransfer, let ch = activeChallenge {
            activeChallenge?.busy = true
            activeChallenge?.error = nil
            Task {
                do {
                    let step = try await transfer.submitCode(code: code)
                    // cancelled while the TAN was on its way: the cancel notice owns the sheet
                    guard self.activeTransfer === transfer else { return }
                    self.handleTransfer(step, connection: ch.connection)
                } catch {
                    guard self.activeTransfer === transfer else { return }
                    self.finishTransfer(error: error)
                }
            }
            return
        }
        guard let session = activeSession, let ch = activeChallenge else { return }
        activeChallenge?.busy = true
        activeChallenge?.error = nil
        Task {
            do {
                let step = try await session.submitCode(code: code)
                // "Überspringen" while the code was on its way: drop the late answer
                guard self.activeSession === session else { return }
                self.handle(step, connection: ch.connection)
            } catch {
                guard self.activeSession === session else { return }
                self.finishSync(error: error)
            }
        }
    }

    /// "TAN auslassen" (#1730): leaves out the order the bank wants a TAN for during the
    /// fetch; the sync goes on with the rest and reports the order as a warning.
    func skipOrder() {
        guard let session = activeSession, let ch = activeChallenge, ch.canSkipOrder, !ch.busy else { return }
        pollTask?.cancel()
        activeChallenge?.busy = true
        activeChallenge?.error = nil
        Task {
            do {
                let step = try await session.skip()
                guard self.activeSession === session else { return }
                self.handle(step, connection: ch.connection)
            } catch {
                guard self.activeSession === session else { return }
                self.finishSync(error: error)
            }
        }
    }

    func cancelSync() {
        pollTask?.cancel()
        if let t = activeTransfer {
            activeTransfer = nil
            activeChallenge = nil
            transferBusy = false
            transferError = nil
            // A collection run or a batch: its own sheet comes back (lines and rows are still there),
            // and the flag must not make the next single transfer end as a batch or a collection.
            if activeTransferIsDebit {
                activeTransferIsDebit = false
                debitRun?.pin = ""
                debitRun = nil
                mandateError = String(localized: "Einzug abgebrochen – die TAN wurde nicht bestätigt. Bereits freigegebene Dateien bitte unter „Geplant“ prüfen, bevor Sie erneut einreichen.")
                showCollectionSheet = true
                mandatesVersion += 1
                cancelTransferSession(t, confirmedMessage: nil)
                return
            }
            if activeTransferIsBatch {
                activeTransferIsBatch = false
                batchDraft.pin = ""
                transferError = String(localized: "Sammelüberweisung abgebrochen – es wurde nichts gesendet, solange die TAN nicht bestätigt wurde.")
                showBatchSheet = true
                cancelTransferSession(t, confirmedMessage: nil)
                return
            }
            if activeTransferIsStanding {
                // not a transfer: the standing-order sheet comes back, not the transfer form
                cancelStandingOrderRun()
                cancelTransferSession(t, confirmedMessage: nil)
                return
            }
            // The bank has to confirm: a bunq draft stays approvable until it is rejected.
            cancelTransferSession(t, confirmedMessage: String(localized: "Überweisung abgebrochen – es wurde nichts gesendet, solange die TAN nicht bestätigt wurde."))
            return
        }
        let s = activeSession
        if s != nil, let id = syncingConnectionID {
            logSync(connectionID: id, source: syncLogSource(for: id), outcome: .cancelled,
                    message: String(localized: "Vom Benutzer abgebrochen"))
        }
        activeSession = nil
        activeChallenge = nil
        pendingLogin = nil
        clearSyncTargets()
        syncGeneration &+= 1
        let generation = syncGeneration
        // "Alle abbrechen": the window stays and shows the rest as skipped
        syncQueue = []
        let inRun = syncRunActive
        syncRun?.skipRemaining(reason: Self.syncRunCancelled)
        syncRunBrowserImports = [:]
        guard inRun, s != nil else {
            syncingConnectionID = nil
            Task { await s?.cancel() }
            // no bank dialog to wait for (a PIN or setup prompt was open): the run is over now
            if inRun { syncNextInQueue() }
            return
        }
        // As in skipCurrentSync: the bank dialog ends first, so an "Erneut versuchen" in the
        // window (queued meanwhile) never meets it still open at the same bank.
        Task {
            await s?.cancel()
            guard self.syncGeneration == generation else { return }
            self.syncingConnectionID = nil
            self.syncNextInQueue()
        }
    }

    func finishSync(error: Error) {
        let failedID = quietSyncConnectionID ?? syncingConnectionID
        logSync(connectionID: failedID, source: syncLogSource(for: failedID), outcome: .error, message: describe(error))
        let retrySecret = failedSyncSecret
        failedSyncSecret = nil
        if let quietID = quietSyncConnectionID {
            // scheduled sync: a notice instead of dialogs / login prompts
            quietSyncConnectionID = nil
            pollTask?.cancel()
            activeSession = nil
            activeChallenge = nil
            syncingConnectionID = nil
            let name = connections.first { $0.id == quietID }?.displayName ?? ""
            toast = String(localized: "\(name): automatischer Abruf fehlgeschlagen – \(describe(error))")
            autoSyncStatus.failureDetail = describe(error)
            return
        }
        statementsAccountID = nil
        postboxConnectionID = nil
        // syncAccountID stays: the retry and the prompts below keep to the account
        syncingConnectionIDBeforeError = syncingConnectionID
        pollTask?.cancel()
        activeSession = nil
        activeChallenge = nil
        syncingConnectionID = nil
        // Trade Republic or Kontist on its stored session alone (no PIN given, none stored) whose
        // session expired (#1128): the core asks for the PIN/password, which only the PIN sheet
        // takes (for Trade Republic it also offers the browser login). FNZ: a failed direct
        // login with a saved PIN — rejected sign-in or changed portal — gets the same sheet
        // with the browser fallback (#1863), otherwise no window could reach it again.
        if let ce = error as? CoreError, retrySecret?.isEmpty ?? true,
           let conn = connections.first(where: { $0.id == syncingConnectionIDBeforeError }) {
            if [.tradeRepublic, .kontist].contains(conn.connector),
               case .Authentication = ce, connectionStatus(conn.id)?.hasPin != true {
                toast = describe(error)
                pendingLogin = PendingLogin(connection: conn)
                return
            }
            if conn.connector == .fnz {
                var fallback = false
                if case .Authentication = ce { fallback = true }
                if case .Protocol = ce { fallback = true }
                if fallback {
                    toast = describe(error)
                    pendingLogin = PendingLogin(connection: conn)
                    return
                }
            }
        }
        // "Alle abrufen" (#1041): an error ends only this login. Its row in the sync window
        // shows it with "Erneut versuchen", and the run goes on with the next login.
        if syncRunActive, !Self.errorOpensPrompt(error),
           let conn = connections.first(where: { $0.id == syncingConnectionIDBeforeError }) {
            syncRun?.fail(conn.id, message: describe(error))
            // without the PIN: the window may keep these until "Fertig"; a retry asks for it the normal way
            let alternatives = productIDAlternatives(for: conn, error: error, secret: nil)
            syncRunActions[conn.id] = alternatives.isEmpty ? nil : alternatives
            syncNextInQueue()
            return
        }
        // A prompt (browser login, setup form, retry with GiroCrab's registration) keeps
        // the batch: done, it syncs this login and goes on; cancelled, it skips it.
        if let ce = error as? CoreError, case .BrowserLoginRequired(let m) = ce,
           let conn = connections.first(where: { $0.id == syncingConnectionIDBeforeError }) {
            toast = m
            webLoginConnection = conn
            return
        }
        if Self.isMissingProductID(error),
           let conn = connections.first(where: { $0.id == syncingConnectionIDBeforeError }) {
            // Never a setup form for the optional FinTS registration: it would be an empty,
            // seemingly mandatory field. Fall back to GiroCrab's registration (once), else offer the own ID.
            if let vault, recoverFromMissingProductID(vault) {
                requestSync(conn)
                return
            }
            if syncRunActive {
                syncRun?.fail(conn.id, message: String(localized: "Diese Version enthält keine FinTS-Registrierung von GiroCrab. Tragen Sie Ihre eigene Registrierung ein."))
                syncRunActions[conn.id] = [(String(localized: "Eigene Registrierung eintragen …"), { [weak self] in self?.enterOwnProductID() })]
                syncNextInQueue()
                return
            }
            resetSyncBatch()
            showError(String(localized: "Abruf von „\(conn.displayName)“ fehlgeschlagen"),
                      String(localized: "Diese Version enthält keine FinTS-Registrierung von GiroCrab. Tragen Sie Ihre eigene Registrierung ein."))
            errorAlternatives = [(String(localized: "Eigene Registrierung eintragen …"), { [weak self] in self?.enterOwnProductID() })]
            return
        }
        if let ce = error as? CoreError, case .MissingSetting(let key) = ce,
           let conn = connections.first(where: { $0.id == syncingConnectionIDBeforeError }) {
            var keys = missingSettings(for: conn)
            if !keys.contains(key) { keys.insert(key, at: 0) }
            setupRequest = SetupRequest(connection: conn, keys: keys)
            return
        }
        if syncRunActive {
            // no login to name: the running one of the run failed
            if let id = syncRun?.running?.connectionID { syncRun?.fail(id, message: describe(error)) }
            syncNextInQueue()
            return
        }
        resetSyncBatch()
        if let conn = connections.first(where: { $0.id == syncingConnectionIDBeforeError }) {
            showError(String(localized: "Abruf von „\(conn.displayName)“ fehlgeschlagen"), describe(error)) { [weak self] in
                guard let self, let c = self.connections.first(where: { $0.id == conn.id }) else { return }
                self.requestSync(c)
            }
            errorAlternatives = productIDAlternatives(for: conn, error: error, secret: retrySecret)
        } else {
            showError(String(localized: "Abruf fehlgeschlagen"), describe(error))
        }
    }
}

extension AppModel {
    /// Opens the account choice for the first login whose new accounts overlap with
    /// another login (#346). No-op while a choice is already shown.
    func checkPendingAccountSelection() {
        guard accountSelectionConnection == nil else { return }
        Task { [weak self] in
            guard let self,
                  let ids = try? await self.vaultOffMain({ try $0.pendingAccountSelectionConnections() }),
                  let first = ids.first,
                  self.accountSelectionConnection == nil else { return }
            self.accountSelectionConnection = self.connections.first { $0.id == first }
        }
    }
}

extension AppModel {
    // MARK: - Sync window (SyncRunSheet)

    /// The login's current name, or the one it had when the run started.
    func connectionName(_ entry: SyncRunEntry) -> String {
        connections.first { $0.id == entry.connectionID }?.displayName ?? entry.name
    }

    /// The sync window shows a prompt of the running login below its list.
    var syncRunPromptShown: Bool { pendingLogin != nil || setupRequest != nil || activeChallenge != nil }

    /// Reason in the sync window for logins "Alle abbrechen" left out.
    static var syncRunCancelled: String { String(localized: "Abruf abgebrochen") }

    /// Skipped by the user or by "Alle abbrechen": worth another try (a lock is not).
    static func retryableSkip(_ reason: String) -> Bool {
        reason == skippedByUser || reason == syncRunCancelled || reason == browserClosed
    }

    /// Reason for a login whose browser window closed without a fetch.
    static var browserClosed: String { String(localized: "Browser-Fenster ohne Abruf geschlossen") }

    /// What the running login waits for, for its row.
    func syncRunPromptText(for id: Int64) -> String {
        if pendingLogin?.connection.id == id { return String(localized: "Anmeldung erforderlich") }
        if setupRequest?.connection.id == id { return String(localized: "Angaben fehlen") }
        if webLoginConnection?.id == id { return String(localized: "Anmeldung im Browser-Fenster") }
        if let ch = activeChallenge, ch.connection.id == id {
            switch ch.challenge.kind {
            case .decoupledPush: return String(localized: "Wartet auf Freigabe")
            case .progress: return String(localized: "Wird abgerufen …")
            default: return String(localized: "TAN erforderlich")
            }
        }
        return String(localized: "Wird abgerufen …")
    }

    /// `--demo-sheet sync-run[-done]` (screenshots): the sync window with fictional results,
    /// one login per status; nothing is fetched.
    func presentDemoSyncRun(finished: Bool) {
        let logins = connections.filter { !$0.allAccountsArchived }
        guard logins.count >= 2 else { return }
        var run = SyncRun(logins: logins.map { ($0.id, $0.displayName) })
        // running: fetched, fetched, the running one, waiting …; over: fetched, failed, skipped, fetched …
        for (i, c) in logins.enumerated() {
            switch (finished, i) {
            case (_, 0): run.finish(c.id, imported: 12, detail: nil)
            case (false, 1): run.finish(c.id, imported: 0, detail: nil)
            case (false, 2): run.start(c.id)
            case (true, 1): run.fail(c.id, message: String(localized: "Die Bank hat nicht geantwortet (Zeitüberschreitung)."))
            case (true, 2): run.skip(c.id, reason: Self.skippedByUser)
            case (true, _): run.finish(c.id, imported: 3, detail: nil)
            default: break
            }
        }
        syncRun = run
    }
}
