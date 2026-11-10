import SwiftUI
import GiroCrabCore

// MARK: - Drag payloads (plain strings, so no custom UTType is needed)

enum GroupDrag {
    static let accountPrefix = "girocrab-account:"
    static let groupPrefix = "girocrab-group:"

    static func account(_ id: Int64) -> String { accountPrefix + String(id) }
    static func group(_ uuid: String) -> String { groupPrefix + uuid }

    static func accountID(_ s: String) -> Int64? {
        s.hasPrefix(accountPrefix) ? Int64(s.dropFirst(accountPrefix.count)) : nil
    }

    static func groupUUID(_ s: String) -> String? {
        s.hasPrefix(groupPrefix) ? String(s.dropFirst(groupPrefix.count)) : nil
    }
}

// MARK: - Sidebar pieces

/// "Konten" title with the view switch and group actions.
struct AccountsSectionHeader: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        HStack(spacing: 6) {
            Text("Konten")
                .font(.caption.weight(.semibold))
                .foregroundStyle(Theme.secondary)
            Spacer()
            Menu {
                Picker("Darstellung", selection: $model.accountView) {
                    ForEach(AccountSidebarMode.allCases) { m in
                        Label(m.title, systemImage: m.symbol).tag(m)
                    }
                }
                .pickerStyle(.inline)
                Divider()
                Button("Neue Gruppe …", systemImage: "folder.badge.plus") { model.groupDialog = .create() }
                Button("Gruppen bearbeiten …", systemImage: "slider.horizontal.3") { model.showGroupEditor = true }
            } label: {
                Label(model.accountView == .byGroup ? "Nach Gruppe" : "Nach Kontoart", systemImage: "line.3.horizontal.decrease")
                    .labelStyle(.titleAndIcon)
                    .font(.caption)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .foregroundStyle(Theme.secondary)
            .accessibilityLabel("Kontenansicht und Gruppen")
        }
        .padding(.top, 12)
    }
}

/// Header of a user group: chevron, symbol, name, sum.
struct UserGroupHeader: View {
    let group: UserGroup
    var open: Bool = true
    var toggle: () -> Void = {}

    var body: some View {
        HStack(spacing: 5) {
            Button(action: toggle) {
                Image(systemName: "chevron.right")
                    .font(.system(size: 9, weight: .bold))
                    .rotationEffect(.degrees(open ? 90 : 0))
                    .frame(width: 12, height: 14)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .foregroundStyle(Theme.tertiary)
            .accessibilityLabel(open ? "Gruppe einklappen" : "Gruppe ausklappen")
            Image(systemName: group.sfSymbol)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(Theme.tertiary)
                .frame(width: 14)
            Text(group.name).lineLimit(1)
            if !group.includeInTotal {
                Image(systemName: "eye.slash")
                    .font(.system(size: 9))
                    .foregroundStyle(Theme.tertiary)
                    .help("Nicht in der Gesamtsumme")
                    .accessibilityLabel("Nicht in der Gesamtsumme")
            }
            Spacer()
            Text(Money.format(group.balanceCents))
                .monospacedDigit()
                .foregroundStyle(group.balanceCents < 0 ? Theme.negative : Theme.secondary)
                .opacity(group.includeInTotal ? 1 : 0.6)
        }
        .font(.caption.weight(.semibold))
        .foregroundStyle(Theme.secondary)
    }
}

/// "Gruppe ▸" submenu for an account (both sidebar views).
struct AccountGroupMenu: View {
    @Environment(AppModel.self) private var model
    let account: Account

    var body: some View {
        Menu("Gruppe", systemImage: "folder") {
            ForEach(model.namedUserGroups) { g in
                Toggle(isOn: Binding(
                    get: { account.userGroupUuid == g.uuid },
                    set: { _ in model.moveAccount(account.id, to: g.uuid) }
                )) {
                    Label(g.name, systemImage: g.sfSymbol)
                }
            }
            Toggle(isOn: Binding(
                get: { account.userGroupUuid == nil },
                set: { _ in model.moveAccount(account.id, to: nil) }
            )) {
                Label("Ohne Gruppe", systemImage: "tray")
            }
            Divider()
            Button("Neue Gruppe …", systemImage: "folder.badge.plus") { model.groupDialog = .create(movingAccount: account.id) }
        }
    }
}

/// Context menu of a user group header.
struct UserGroupMenu: View {
    @Environment(AppModel.self) private var model
    let group: UserGroup

    var body: some View {
        #if os(iOS)
        Button("Gruppenübersicht", systemImage: "list.bullet.rectangle") { model.sidebarSelection = .userGroup(group.uuid) }
        #endif
        if !group.isUngrouped {
            Button("Umbenennen …", systemImage: "pencil") { model.groupDialog = .rename(group) }
            Menu("Symbol", systemImage: group.sfSymbol) {
                ForEach(GroupSymbols.named, id: \.symbol) { item in
                    Toggle(isOn: Binding(
                        get: { item.symbol == group.sfSymbol },
                        set: { _ in model.updateUserGroup(group, symbol: item.symbol) }
                    )) {
                        Label(item.name, systemImage: item.symbol)
                    }
                }
            }
            Toggle(isOn: Binding(
                get: { group.includeInTotal },
                set: { model.updateUserGroup(group, includeInTotal: $0) }
            )) {
                Label("In Gesamtsumme einbeziehen", systemImage: "sum")
            }
        }
        Divider()
        Button("Neue Gruppe …", systemImage: "folder.badge.plus") { model.groupDialog = .create() }
        Button("Gruppen bearbeiten …", systemImage: "slider.horizontal.3") { model.showGroupEditor = true }
        if !group.isUngrouped {
            Divider()
            Button("Gruppe löschen …", systemImage: "trash", role: .destructive) { model.groupDialog = .delete(group) }
        }
    }
}

// MARK: - Dialogs (one place for all group dialogs)

/// Presents `model.groupDialog` and the editor sheet. Attached to the main
/// window and inside the editor sheet; only one of them presents at a time.
struct AccountGroupDialogs: ViewModifier {
    @Environment(AppModel.self) private var model
    @State private var text = ""
    /// Attached inside the editor sheet: that one presents while the sheet
    /// is up, the window's one otherwise.
    var inEditor = false

    private var hosts: Bool { model.showGroupEditor == inEditor }

    /// Shown while the dialog is of this kind; dismissing clears it unless
    /// an action already switched to another dialog.
    private func presented(text wantsText: Bool) -> Binding<Bool> {
        Binding(
            get: { hosts && model.groupDialog?.needsText == wantsText },
            set: { if !$0, model.groupDialog?.needsText == wantsText { model.groupDialog = nil } }
        )
    }

    func body(content: Content) -> some View {
        @Bindable var model = model
        content
            .alert(model.groupDialog?.title ?? Text(verbatim: ""), isPresented: presented(text: true), presenting: model.groupDialog) { d in
                TextField("Name, z. B. „Privat“ oder „Geschäft“", text: $text)
                Button(d.isRename ? "Umbenennen" : "Anlegen") { submit(d) }
                Button("Abbrechen", role: .cancel) {}
            } message: { d in
                if case .create(let id?) = d, let a = model.account(for: id) {
                    Text("„\(a.name)“ wird in die neue Gruppe verschoben.")
                }
            }
            .confirmationDialog(
                model.groupDialog?.title ?? Text(verbatim: ""),
                isPresented: presented(text: false),
                titleVisibility: .visible,
                presenting: model.groupDialog
            ) { d in
                switch d {
                case .delete(let g):
                    Button("Löschen", role: .destructive) { model.deleteUserGroup(g) }
                case .moveAccount(let a):
                    ForEach(model.namedUserGroups) { g in
                        Button { model.moveAccount(a.id, to: g.uuid) } label: {
                            Text(verbatim: a.userGroupUuid == g.uuid ? "✓ \(g.name)" : g.name)
                        }
                    }
                    Button { model.moveAccount(a.id, to: nil) } label: {
                        a.userGroupUuid == nil ? Text(verbatim: "✓ ") + Text("Ohne Gruppe") : Text("Ohne Gruppe")
                    }
                    Button("Neue Gruppe …") {
                        // after this dialog has closed
                        DispatchQueue.main.async { model.groupDialog = .create(movingAccount: a.id) }
                    }
                case .create, .rename:
                    EmptyView()
                }
                Button("Abbrechen", role: .cancel) {}
            } message: { d in
                if case .delete(let g) = d { Text(deleteMessage(g)) }
            }
            .onChange(of: model.groupDialog?.initialText) { _, t in text = t ?? "" }
            .sheet(isPresented: inEditor ? .constant(false) : $model.showGroupEditor) {
                GroupEditorView()
                    .environment(model)
            }
    }

    private func submit(_ d: GroupDialog) {
        let name = text.trimmingCharacters(in: .whitespaces)
        guard !name.isEmpty else { return }
        switch d {
        case .create(let accountID): model.createUserGroup(name: name, moving: accountID)
        case .rename(let g): model.updateUserGroup(g, name: name)
        case .delete, .moveAccount: break
        }
    }
}

extension View {
    func accountGroupDialogs(inEditor: Bool = false) -> some View { modifier(AccountGroupDialogs(inEditor: inEditor)) }
}

/// "Gruppen bearbeiten …": order, rename, symbol, total, members.
struct GroupEditorView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var path: [String] = []

    var body: some View {
        NavigationStack(path: $path) {
            List {
                Section {
                    ForEach(model.namedUserGroups) { g in
                        NavigationLink(value: g.uuid) {
                            HStack(spacing: 10) {
                                Image(systemName: g.sfSymbol).foregroundStyle(Theme.accent).frame(width: 22)
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(g.name)
                                    Text(g.accounts.isEmpty ? String(localized: "Keine Konten") : String(localized: "\(g.accounts.count) Konten"))
                                        .font(.caption).foregroundStyle(Theme.secondary)
                                }
                                Spacer()
                                if !g.includeInTotal {
                                    Image(systemName: "eye.slash").foregroundStyle(Theme.tertiary).help("Nicht in der Gesamtsumme")
                                }
                                Text(Money.format(g.balanceCents)).monospacedDigit().foregroundStyle(Theme.secondary)
                            }
                        }
                        .contextMenu {
                            Button("Gruppe löschen …", systemImage: "trash", role: .destructive) { model.groupDialog = .delete(g) }
                        }
                    }
                    .onMove { model.moveUserGroups(from: $0, to: $1) }
                    .onDelete { idx in model.groupDialog = idx.first.map { .delete(model.namedUserGroups[$0]) } }
                } header: {
                    Text("Gruppen")
                } footer: {
                    Text(model.hasUserGroups
                         ? "Reihenfolge per Ziehen ändern. Konten ohne Gruppe erscheinen unter „Ohne Gruppe“."
                         : "Mit Gruppen trennen Sie z. B. private und geschäftliche Konten oder die Konten von Familienmitgliedern.")
                }
                Section {
                    Button("Neue Gruppe …", systemImage: "folder.badge.plus") { model.groupDialog = .create() }
                }
            }
            .navigationTitle("Kontogruppen")
            .navigationDestination(for: String.self) { uuid in
                GroupDetailEditor(uuid: uuid)
            }
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { dismiss() }
                }
                #if os(iOS)
                ToolbarItem(placement: .topBarLeading) { EditButton() }
                #endif
            }
        }
        // group dialogs must also work while this sheet is up
        .accountGroupDialogs(inEditor: true)
        #if os(macOS)
        .frame(minWidth: 480, minHeight: 540)
        #endif
    }
}

func deleteMessage(_ g: UserGroup) -> String {
    g.accounts.isEmpty
        ? String(localized: "Die Gruppe ist leer.")
        : String(localized: "\(g.accounts.count) Konten werden nach „Ohne Gruppe“ verschoben. Es werden keine Konten oder Buchungen gelöscht.")
}

/// One group: name, symbol, total switch and which accounts belong to it.
struct GroupDetailEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let uuid: String
    @State private var name = ""

    private var group: UserGroup? { model.userGroups.first { $0.uuid == uuid } }

    var body: some View {
        if let g = group {
            Form {
                Section("Name") {
                    TextField("Name", text: $name)
                        .onSubmit { commitName() }
                }
                Section("Symbol") {
                    LazyVGrid(columns: [GridItem(.adaptive(minimum: 40), spacing: 8)], spacing: 8) {
                        ForEach(GroupSymbols.all, id: \.self) { s in
                            Button { model.updateUserGroup(g, symbol: s) } label: {
                                Image(systemName: s)
                                    .font(.system(size: 16))
                                    .frame(width: 38, height: 38)
                                    .background(RoundedRectangle(cornerRadius: 8).fill(s == g.sfSymbol ? Theme.accent.opacity(0.18) : Color.clear))
                                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(s == g.sfSymbol ? Theme.accent : Theme.tertiary.opacity(0.3)))
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .foregroundStyle(s == g.sfSymbol ? Theme.accent : Theme.secondary)
                            .accessibilityLabel(s)
                            .accessibilityAddTraits(s == g.sfSymbol ? .isSelected : [])
                        }
                    }
                    .padding(.vertical, 4)
                }
                Section {
                    Toggle("In Gesamtsumme einbeziehen", isOn: Binding(
                        get: { g.includeInTotal },
                        set: { model.updateUserGroup(g, includeInTotal: $0) }
                    ))
                } footer: {
                    Text("Ausgeschaltet zählen die Konten dieser Gruppe nicht zu „Verfügbar“ und zum Saldoverlauf der Übersicht – z. B. für Geschäfts- oder Kinderkonten.")
                }
                Section {
                    ForEach(model.allAccounts) { a in
                        let member = a.userGroupUuid == g.uuid
                        Button {
                            model.moveAccount(a.id, to: member ? nil : g.uuid)
                        } label: {
                            HStack(spacing: 9) {
                                AccountSignet(account: a, size: 22)
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(a.name).foregroundStyle(Theme.text)
                                    if !member, let other = model.userGroup(of: a) {
                                        Text("Jetzt in „\(other.name)“").font(.caption2).foregroundStyle(Theme.secondary)
                                    } else {
                                        Text(a.bankName).font(.caption2).foregroundStyle(Theme.secondary)
                                    }
                                }
                                Spacer()
                                Image(systemName: member ? "checkmark.circle.fill" : "circle")
                                    .foregroundStyle(member ? Theme.accent : Theme.tertiary)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(member ? .isSelected : [])
                    }
                } header: {
                    Text("Konten")
                } footer: {
                    Text("Ein Konto gehört zu höchstens einer Gruppe.")
                }
                Section {
                    Button("Gruppe löschen …", role: .destructive) { model.groupDialog = .delete(g) }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(g.name)
            .onAppear { name = g.name }
            .onDisappear { commitName() }
        } else {
            // deleted (here or via sync) → back to the list
            Color.clear.onAppear { dismiss() }
        }
    }

    /// Saves the edited name, if the group still exists.
    private func commitName() {
        let n = name.trimmingCharacters(in: .whitespaces)
        guard let g = group, !n.isEmpty, n != g.name else { return }
        model.updateUserGroup(g, name: n)
    }
}
