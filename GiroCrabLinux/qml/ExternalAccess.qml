pragma Singleton
import QtQuick
import GiroCrab

// AppModel+LocalApi.swift (#136): read-only local interface for external analysis. The socket
// (`LocalApiServer` in the core, src/local_api.rs) runs only while the personal vault is
// unlocked and shown, never for the demo, and only when the user enabled it and created an
// access token. Settings UI: ExternalAccessSettings.qml (Einstellungen › Analyse).
// Linux: the socket lives in $XDG_RUNTIME_DIR (src/local_api.rs › socket_path); the core
// closes it on lock and demo itself (dispatch.rs), the app on quit (main.rs).
QtObject {
    id: api

    /// `localApiScope`: ShareScope of the core (snake_case), null = not loaded / no target.
    property var scope: null
    property bool scopeReadFailed: false
    property int sessionGeneration: 0
    /// `localApiNewToken`: shown once, right after it was created ("" = none). Never logged.
    property string newToken: ""
    /// `isLocalApiRunning`
    property bool running: false
    /// `localApiStartError` ("" = none)
    property string startError: ""
    property string socketPath: ""
    /// `localApiHelperURL` (girocrab-mcp), "" = not built
    property string helperPath: ""
    /// `#if DEBUG`
    property bool devBuild: false

    readonly property bool hasToken: !!scope && (scope.token_sha256 || "") !== ""

    /// `localApiTarget`: the own vault, unlocked and shown (not the demo, not behind the lock
    /// screen while the unlock still loads). The one condition every start, stop and settings
    /// call checks.
    readonly property bool target: AppModel.holdsRealData && AppModel.vaultState === "open"
    onTargetChanged: reconcile()

    /// `reconcileLocalApiServer`: stopping is immediate, starting follows the queued scope read.
    function reconcile() {
        if (!target) {
            stop()
            // Locked, demo or unlocking: a key shown once is never shown again.
            scope = null
            scopeReadFailed = false
            sessionGeneration++
            newToken = ""
            return
        }
        job("local_api_reconcile", {}, null)
    }

    /// `stopLocalApiServer`
    function stop() {
        AppModel.coreOrNull("local_api_stop")
        running = false
        startError = ""
    }

    /// Coming back to the app (another instance may have quit meanwhile): `retryLocalApiStart`.
    property Connections activation: Connections {
        target: Qt.application
        function onStateChanged() {
            if (Qt.application.state === Qt.ApplicationActive && api.startError !== "") api.reconcile()
        }
    }

    // MARK: job queue (`localApiJob`): every call of this feature, one after another on the
    // vault as it was when the call was made; each ends by matching socket and scope to what
    // is stored. While newer jobs wait, only the token is taken over, so quick toggles do not
    // flicker back; the newest job publishes it all (and so undoes a save that failed).
    property var queue: []
    property bool busy: false
    property int jobs: 0

    function job(method, args, publish) {
        if (!target) return
        jobs++
        queue.push({ method: method, args: args, publish: publish, number: jobs, gen: AppModel.vaultGeneration })
        if (!busy) next()
    }
    function next() {
        const j = queue.shift()
        if (!j) { busy = false; return }
        busy = true
        // locked or swapped before the call's turn
        if (!target || j.gen !== AppModel.vaultGeneration) { next(); return }
        AppModel.coreAsync(j.method, j.args, r => {
            if (target && j.gen === AppModel.vaultGeneration) {
                apply(r.state, j.number === jobs)
                if (r.error) AppModel.errorMessage = r.error
                else if (j.publish) j.publish(r.value)
            }
            next()
        }, () => next())   // no target any more: nothing to show
    }

    /// `applyLocalApiScope`
    function apply(state, isNewest) {
        running = state.running
        startError = state.start_error || ""
        socketPath = state.socket_path || ""
        helperPath = state.helper_path || ""
        devBuild = state.dev_build
        const s = state.scope
        if (!s) {
            scopeReadFailed = true
            return   // unreadable: start error set
        }
        scopeReadFailed = false
        // Another token (revoked, replaced): the one shown is no longer valid.
        if (s.token_sha256 !== (scope ? scope.token_sha256 : "")) newToken = ""
        if (isNewest || !scope) {
            scope = s
        } else {
            const c = Object.assign({}, scope)
            c.token_sha256 = s.token_sha256
            scope = c
        }
    }

    /// `updateLocalApiScope`: a switch in the settings pane, shown at once, saved in the queue.
    function updateScope(s) {
        if (!target) return
        scope = shownScope(s)
        job("set_local_api_scope", { scope: s }, null)
    }
    /// `shownScope`: switching external access off revokes the key in the core (#811), so the
    /// pane no longer offers one.
    function shownScope(s) {
        if (!(scope && scope.enabled) || s.enabled) return s
        const shown = Object.assign({}, s)
        shown.token_sha256 = ""
        newToken = ""
        return shown
    }
    /// `createLocalApiToken`: a new access token, shown once; the previous one stops working.
    function createToken() {
        const sGen = sessionGeneration
        job("create_local_api_token", {}, token => {
            if (sGen === api.sessionGeneration) api.newToken = token
        })
    }
    function revokeToken() { job("revoke_local_api_token", {}, null) }
    /// `forgetNewLocalApiToken`: the pane closed, the new key is not shown again.
    function forgetNewToken() {
        sessionGeneration++
        newToken = ""
    }
}
