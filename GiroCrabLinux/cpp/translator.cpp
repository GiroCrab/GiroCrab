// UI language (LanguagePreference.swift): German source texts, English from the Mac app's
// string catalog (build.rs → OUT_DIR/en.json, looked up in src/i18n.rs). The choice
// ("system", "de", "en") lives in QSettings (key `language`, ~/.config/GiroCrab/GiroCrab.conf)
// and takes effect after a restart, so one run never mixes languages.
#include <QtCore/QCoreApplication>
#include <QtCore/QLocale>
#include <QtCore/QSettings>
#include <QtCore/QTranslator>

// src/i18n.rs: English for a German source text, null if there is none.
extern "C" const char *girocrab_tr(const char *german);

namespace {

const QString kLanguageKey = QStringLiteral("language");

// qsTr / QCoreApplication::translate: the context is ignored, the German text is the key.
// A text without English stays as written: our German source, or the English source of
// Qt's own texts ("Ctrl" in shortcuts), which the desktop's Qt translations (KDE loads
// qtbase_de for a German system) would otherwise turn into German.
class CatalogTranslator : public QTranslator
{
public:
    using QTranslator::QTranslator;
    QString translate(const char *, const char *sourceText, const char *, int) const override
    {
        if (!sourceText)
            return QString();
        const char *en = girocrab_tr(sourceText);
        return QString::fromUtf8(en ? en : sourceText);
    }
    bool isEmpty() const override { return false; }
};

} // namespace

// LanguagePreference.choice: "system", "de" or "en".
extern "C" const char *girocrab_language_choice()
{
    const QString v = QSettings().value(kLanguageKey).toString();
    if (v == QLatin1String("de"))
        return "de";
    if (v == QLatin1String("en"))
        return "en";
    return "system";
}

// LanguagePreference.apply: stored at once (the restart reads it).
extern "C" void girocrab_set_language_choice(const char *choice)
{
    QSettings settings;
    const QString v = QString::fromUtf8(choice);
    if (v == QLatin1String("de") || v == QLatin1String("en"))
        settings.setValue(kLanguageKey, v);
    else
        settings.remove(kLanguageKey);
    settings.sync();
}

// LanguagePreference.systemLanguage: the first entry of the desktop's language list
// (LANGUAGE, LC_ALL, LC_MESSAGES, LANG) the app speaks. A system in neither language gets
// English (the Mac asks with a language screen before the vault opens; not ported).
extern "C" const char *girocrab_system_language()
{
    for (const QString &tag : QLocale::system().uiLanguages()) {
        const QString code = tag.section(QLatin1Char('-'), 0, 0).section(QLatin1Char('_'), 0, 0).toLower();
        if (code == QLatin1String("de"))
            return "de";
        if (code == QLatin1String("en"))
            return "en";
    }
    return "en";
}

// Called once before the QML engine loads. English: the catalog translator. Both: the
// default QLocale (Qt.locale() in Fmt.qml) speaks the UI language with the system's region,
// like Locale.autoupdatingCurrent on the Mac ("en_DE" for English on a German system).
extern "C" void girocrab_apply_language(const char *code)
{
    const bool english = qstrcmp(code, "en") == 0;
    if (english)
        QCoreApplication::installTranslator(new CatalogTranslator(QCoreApplication::instance()));
    const QLocale system = QLocale::system();
    const QLocale::Language language = english ? QLocale::English : QLocale::German;
    QLocale locale(language, system.territory());
    if (locale.language() != language)
        locale = QLocale(language);
    QLocale::setDefault(locale);
}
