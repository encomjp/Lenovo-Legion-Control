# Datenschutzerklärung — Telemetrie der Alpha-Version (Legion Control)

**Stand:** 25. August 2026 · **Gültig für:** Legion Control ab Build 0.1.0-alpha

## 1. Wer ist verantwortlich?

Verantwortlicher für die Datenverarbeitung ist der Betreiber des
Telemetrie-Servers (Adrian Kozlowski, Mannheim — vollständige Kontaktdaten
im Impressum der Hauptwebsite). Die Verarbeitung erfolgt ausschließlich auf
einer Server-Infrastruktur bei IONOS (Rechenzentrum Deutschland).

## 2. Was wird gesammelt?

Nur wenn du im **Erststart-Dialog** „Share ✓" wählst oder später in der App
unter **Einstellungen → Setup → „Share anonymous diagnostics"** ausdrücklich
aktivierst, übermittelt die Software **einen** anonymisierten JSON-Bericht
pro Klick auf „Send now":

- **Gerät:** Modellbezeichnung (z. B. „Legion Pro 7 16AFR10H"), Maschinentyp,
  BIOS-Version, CPU-/GPU-Modell, EC-Chip
- **Betriebssystem:** Distribution und Kernel-Version
- **Sensoren:** Temperaturen, Lüfter-Drehzahlen und -Ziele, Lüftergrenzen,
  Akkuzustand (Kapazität, Gesundheit, Zyklen — **keine** Seriennummer)
- **Konfiguration:** Thermallimit, Curve-Optimizer-Status (nur Werte),
  Lüfterprofil-Art, Energieschema
- **App-Zustand:** Einstellungen-Digest (Beleuchtungsmodus, Tastaturlayout)
  und eine **Log-Zusammenfassung** (Warn-/Fehlerzähler plus die letzte
  Fehlermeldung — Home-Pfade technisch geschwärzt). Rohe Logzeilen verlassen
  dein Gerät nicht.
- **Selbsttests:** Ergebnisliste der eingebauten Diagnose

## 3. Was wird NIEMALS gesammelt?

Hostname · Benutzername · IP-Adresse (wird beim Empfang verworfen, nicht
gespeichert) · Seriennummern · MAC-Adressen · Tastatur-Belegungen/Farben ·
eigene Profilnamen · gespeicherte Dateiinhalte.

## 4. Rechtliche Einordnung

Die übermittelten Daten enthalten **keine personenbezogenen Daten** im Sinne
von Art. 4 Nr. DSGVO: Ein Bezug zu einer natürlichen Person ist weder
technisch möglich noch vorgesehen (Erwägungsgrund 26 DSGVO — anonymisierte
Daten). Die DSGVO findet auf Erhebung und Speicherung daher **keine
Anwendung**. Unabhängig davon gilt:

- Die Übermittlung erfolgt **ausschließlich nach ausdrücklicher Opt-in-
  Einwilligung** (Standard: aus). Der Widerruf erfolgt jederzeit über denselben
  Schalter; bereits gesendete Berichte verbleiben bis zum Ablauf der
  Speicherfrist.
- Transportverschlüsselung via HTTPS (Cloudflare-TLS + Origin-Zertifikat).
- Speicherort: IONOS VPS, Zugriff nur für den Betreiber.

## 5. Wie lange wird gespeichert?

Berichte werden automatisch nach **90 Tagen** gelöscht (stündlicher Aufräum-
lauf). Du kannst die Löschung eines bestimmten Berichts auch vorher per
GitHub-Issue anfragen.

## 6. Wie kann ich alles selbst prüfen?

`legion-cli diagnose dump` zeigt exakt den Inhalt, der gesendet würde — vor
dem Senden, lokal in deinem Terminal. Quellcode offen einsehbar:
`src/diagnostics.rs` (Whitelist) und `server/wan/collector.py` (Speicherung).
