import SwiftUI
import GiroCrabCore

/// Link between an embedded add form and the assistant's footer: whether the
/// form is complete, and what "Hinzufügen" does.
@Observable final class AddBankSubmit {
    var canSubmit = false
    @ObservationIgnored var action: () -> Void = {}

    func update(_ ok: Bool, _ action: @escaping () -> Void) {
        self.action = action
        if canSubmit != ok { canSubmit = ok }
    }

    /// Forgets the form shown before: switching to another way to connect must
    /// never leave its "Hinzufügen" behind (it added the PSD2 login instead of
    /// the direct one when the new form did not report itself).
    func reset() { update(false, {}) }
}

/// Add form for a bank with a dedicated connector (bunq, comdirect, Trade
/// Republic, PayPal, American Express, Amazon Visa).
struct KindAddForm: View {
    @Environment(AppModel.self) private var model
    let kind: ConnectorKind
    var onAdded: () -> Void = {}
    /// Inside the "Bank hinzufügen" assistant: no own button; readiness and the
    /// add action go to the assistant's footer.
    var submit: AddBankSubmit? = nil
    @State private var login = ""

    private var canAdd: Bool { !(loginRequired && login.trimmingCharacters(in: .whitespaces).isEmpty) }
    private var loginRequired: Bool { Self.loginRequired(kind) }

    private func commit() {
        model.addConnection(kind, login: kind == .tradeRepublic ? normalizePhoneNumber(input: login) : login)
        login = ""
        onAdded()
    }

    /// Banks that need the login to sign in. For the others (Amazon, Amex,
    /// PayPal, bunq …) the field is only an optional display name.
    static func loginRequired(_ kind: ConnectorKind) -> Bool {
        [.comdirect, .tradeRepublic, .advanzia, .volksbankKasselGoettingen, .direkt1822, .finTsGeneric, .alteLeipziger,
         // their connectors refuse to sign in without it (customer number, user name, token)
         .hanseaticBank, .easybank, .ibkr, .kontist, .volkswagenBank, .uberspace, .fordMoney, .opelDirektbank, .bhw, .monefit, .goAndGrow].contains(kind)
    }

    /// Page where the bank hands out the API credentials GiroCrab asks for.
    static func credentialsLink(_ kind: ConnectorKind) -> (title: LocalizedStringKey, url: URL)? {
        switch kind {
        case .comdirect:
            return ("API-Zugangsdaten bei comdirect erstellen", URL(string: "https://kunde.comdirect.de/itx/oauth/privatkunden")!)
        default:
            return nil
        }
    }

    /// One section: login and the fixed facts as rows, every explanation once
    /// in the footer (HIG: notes belong under the section, not between rows).
    var body: some View {
        if kind == .advanzia { AdvanziaAddForm(onAdded: onAdded, submit: submit) }
        else { standardForm }
    }
    @ViewBuilder private var standardForm: some View {
        if let d = bankCatalog().first(where: { $0.kind == kind }) {
            Section {
                TextField(d.loginLabel, text: $login, prompt: loginRequired ? nil : Text("Optional"))
                    .autocorrectionDisabled()
                    .onChange(of: canAdd, initial: true) { _, ok in submit?.update(ok, commit) }
                LabeledContent("Verfahren", value: d.protocol)
                if !d.bankCode.isEmpty { LabeledContent("Bankleitzahl", value: d.bankCode) }
                if submit == nil {
                    Button("Bank hinzufügen") { commit() }
                        .keyboardShortcut(.defaultAction)
                        .disabled(!canAdd)
                }
            } header: {
                Text("Zugang").appSectionHeader()
            } footer: {
                let missing = model.userRequiredKeys(for: d).filter { key in !(model.settings.first { $0.key == key }?.hasValue ?? false) }
                VStack(alignment: .leading, spacing: 8) {
                    // in the footer, not as a row: a card inside the section's box would be a box in a box (HIG)
                    if model.demoActive { DemoHintBanner(style: .credentials) { login = "demo" } }
                    AddFormFooter(lines: [d.note] + (missing.isEmpty || submit != nil ? [] :
                        [String(localized: "Wird nach dem Hinzufügen abgefragt: \(missing.map { settingLabels[$0] ?? $0 }.joined(separator: ", "))")]),
                                  // in the assistant the link sits under the credential fields ("Einrichten")
                                  link: submit == nil ? Self.credentialsLink(kind) : nil)
                }
            }
        }
    }
}

/// Footer of an add form: the notes, then the hint that an existing login is
/// updated rather than added twice.
struct AddFormFooter: View {
    let lines: [String]
    /// Where the user gets what the form asks for (e.g. API credentials).
    var link: (title: LocalizedStringKey, url: URL)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(lines.filter { !$0.isEmpty }.enumerated()), id: \.offset) { _, l in Text(l) }
            if let link {
                // tint, not the footer's grey: it has to read as a link
                Link(link.title, destination: link.url).foregroundStyle(.tint)
            }
            Text("Gibt es den Zugang schon (gleiche Bank, gleiche Kennung), wird er aktualisiert statt doppelt angelegt.")
        }
        .appFont(.caption).foregroundStyle(.secondary)
        .leadingFooter()
    }
}

/// Selects the key before proceeding to the first bank consent.
struct Psd2AddForm: View {
    @Environment(AppModel.self) private var model
    let bank: BankSearchResult
    let onAdded: () -> Void
    let submit: AddBankSubmit
    @State private var label = ""
    @State private var useOtherKey = false
    @State private var values: [String: String] = [:]
    @State private var starting = false
    @State private var finishing = false
    @State private var consent: Consent?
    @State private var addedConnection: (pin: VaultPin, id: Int64)?
    @State private var failure: String?

    private struct Consent: Identifiable {
        let id = UUID()
        let browser: ConsentBrowser
    }

    private let idKey = "enablebanking.application_id"
    private let pemKey = "enablebanking.private_key_pem"
    private var hasSavedKey: Bool {
        model.settingEntry(idKey)?.hasValue == true && model.settingEntry(pemKey)?.hasValue == true
    }
    private var needsFile: Bool { !hasSavedKey || useOtherKey }
    private var demoLogin: Bool { model.demoActive && AppModel.isMagicDemoLogin(label) }
    private var canAdd: Bool {
        !starting && (demoLogin || !needsFile ||
            (!(values[idKey] ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
             !(values[pemKey] ?? "").isEmpty))
    }

    private func commit() {
        guard canAdd else { return }
        if demoLogin {
            model.addEnableBankingConnection(aspsp: bank.aspsp, country: bank.country, name: bank.name, label: label)
            onAdded()
            return
        }
        failure = nil
        starting = true
    }

    private func startConsent() async {
        defer { starting = false }
        do {
            let (pin, enrollment) = try await model.beginPsd2Enrollment(
                bank: bank, label: label,
                applicationID: needsFile ? values[idKey] : nil,
                pem: needsFile ? values[pemKey] : nil)
            guard !Task.isCancelled, model.isCurrent(pin) else { return }
            guard let url = URL(string: enrollment.authorizationUrl()), url.scheme == "https" else {
                throw CoreError.SessionClosed
            }
            let browser = ConsentBrowser(url: url, redirect: enrollment.redirectUrl()) { callback in
                guard consent != nil, !finishing, model.isCurrent(pin) else { return }
                finishing = true
                Task {
                    defer { finishing = false; consent = nil }
                    do {
                        let id = try await enrollment.finish(callback: callback.absoluteString)
                        if model.isCurrent(pin) { addedConnection = (pin, id) }
                    } catch {
                        failure = describe(error)
                    }
                }
            }
            consent = Consent(browser: browser)
        } catch {
            if !Task.isCancelled { failure = describe(error) }
        }
    }

    var body: some View {
        Section {
            LabeledContent("Land", value: Locale.current.localizedString(forRegionCode: bank.country) ?? bank.country)
            LabeledContent("Verfahren", value: String(localized: "PSD2 über Enable Banking"))
            TextField("Bezeichnung (optional)", text: $label, prompt: Text("z. B. Privatkonto"))
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
                .disabled(starting)
        } header: {
            Text("Zugang").appSectionHeader()
        }
        Section {
            if hasSavedKey {
                Picker("Enable-Banking-Schlüssel", selection: $useOtherKey) {
                    Text("Vorhandenen Schlüssel verwenden").tag(false)
                    Text("Andere PEM-Datei verwenden").tag(true)
                }
                #if os(macOS)
                .pickerStyle(.radioGroup)
                #else
                .pickerStyle(.inline)
                #endif
                .labelsHidden()
                .disabled(starting)
                if !useOtherKey, let id = model.settingEntry(idKey)?.value, !id.isEmpty {
                    LabeledContent("Application-ID", value: "…\(id.suffix(8))")
                        .sensitive()
                }
            }
            if needsFile && !demoLogin {
                EnableBankingSetupSection(values: $values, showGuide: !hasSavedKey, collapsibleGuide: true)
                    .disabled(starting)
            }
            if let failure {
                Label(failure, systemImage: "exclamationmark.circle")
                    .appFont(.callout)
                    .foregroundStyle(Theme.negative)
            }
            if starting { ProgressView() }
        } header: {
            Text("Enable-Banking-Schlüssel").appSectionHeader()
        } footer: {
            VStack(alignment: .leading, spacing: 8) {
                if !bank.tested.isEmpty {
                    Text(bank.tested).appFont(.caption).foregroundStyle(.secondary)
                }
                if model.demoActive { DemoHintBanner(style: .credentials) { label = "demo" } }
            }
        }
        .onChange(of: canAdd, initial: true) { _, ready in submit.update(ready, commit) }
        .task(id: starting) {
            if starting { await startConsent() }
        }
        .capturableSheet(item: $consent, onDismiss: didDismissConsent) { pending in
            VStack(alignment: .leading, spacing: 12) {
                Text("Zugriff bei der Bank freigeben").appSectionHeader()
                ConsentWebView(browser: pending.browser)
                    .desktopFrame(minWidth: 820, idealWidth: 880, minHeight: 420, idealHeight: 620, maxHeight: .infinity)
                HStack {
                    Button("Abbrechen") { consent = nil }
                        .keyboardShortcut(.cancelAction)
                        .disabled(finishing)
                    if finishing { ProgressView() }
                }
            }
            .padding(20)
            .interactiveDismissDisabled(finishing)
        }
    }

    private func didDismissConsent() {
        guard let addedConnection, model.isCurrent(addedConnection.pin) else { return }
        onAdded()
        model.finishAddingPsd2Connection(addedConnection.id)
    }
}
