// man -- the shell's own words about its commands (German, our own texts: the trademark and
// CLI rule of the README): help, man PAGE, CMD --help of the classic tools, which and type
//
//   PAGES                 every command of the shell: name, where it lives ("" for a builtin of
//                         the shell), the call, what it does, its options
//   page_of(name)         the page of a command (aliases: "[" is test, "jfn-sh" is sh)
//   man(name) -> Option<String>     the page as man shows it (NAME, AUFRUF, BESCHREIBUNG,
//                                   OPTIONEN), wrapped at 72 columns
//   usage(name) -> String "Aufruf: ..." with the options (CMD --help)
//   help() -> String      the commands with a line each
//
// The network tools keep their own --help texts (curl, nc, ss, top, tftp, mail ...); their
// pages here are short and point there.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// a page of the manual
pub struct Page {
    pub name: &'static str,
    /// "/usr/bin/grep"; "" for a builtin of the shell
    pub path: &'static str,
    pub call: &'static str,
    /// one line for help
    pub line: &'static str,
    /// more words for the page ("" : the line says it all)
    pub text: &'static str,
    pub opts: &'static [(&'static str, &'static str)],
}

const B: &str = "";

pub const PAGES: &[Page] = &[
    Page { name: "basename", path: "/usr/bin/basename", call: "basename PFAD [ENDUNG]", line: "den letzten Teil eines Pfads ausgeben", text: "Schreibt den Namen hinter dem letzten / (ein / am Ende zählt nicht); mit ENDUNG ohne diese.", opts: &[] },
    Page { name: "cat", path: "/usr/bin/cat", call: "cat [-n] [DATEI ...]", line: "Dateien hintereinander ausgeben", text: "Ohne DATEI oder mit - liest cat, was eine Pipe oder < liefert.", opts: &[("-n", "die Zeilen nummerieren")] },
    Page { name: "cd", path: B, call: "cd [VERZEICHNIS | - | ~]", line: "das Arbeitsverzeichnis wechseln", text: "Ohne Argument geht es nach $HOME, mit - zurück ins vorige Verzeichnis (cd zeigt es dann). Relative Pfade gelten ab dem Arbeitsverzeichnis; die Eingabeaufforderung zeigt es (~ für $HOME).", opts: &[] },
    Page { name: "clear", path: "/usr/bin/clear", call: "clear", line: "den Bildschirm leeren", text: "", opts: &[] },
    Page { name: "cp", path: "/usr/bin/cp", call: "cp QUELLE ZIEL", line: "eine Datei kopieren", text: "Ist ZIEL ein Verzeichnis, landet die Kopie dort unter ihrem Namen.", opts: &[] },
    Page { name: "curl", path: "/usr/bin/curl", call: "curl [OPTIONEN] URL ...", line: "HTTP, FTP, SMTP und POP3 als Client", text: "curl --help zeigt die Optionen.", opts: &[] },
    Page { name: "cut", path: "/usr/bin/cut", call: "cut -f LISTE [-d ZEICHEN] [-s] [DATEI ...] | cut -c LISTE [DATEI ...]", line: "Spalten aus Zeilen schneiden", text: "LISTE besteht aus Nummern und Bereichen ab 1, mit Kommas getrennt: 1,3 oder 2-4 oder 3-.", opts: &[("-f LISTE", "diese Felder"), ("-d ZEICHEN", "Felder trennt dieses Zeichen (sonst ein Tabulator)"), ("-c LISTE", "diese Zeichen"), ("-s", "Zeilen ohne Trennzeichen weglassen")] },
    Page { name: "date", path: "/usr/bin/date", call: "date [-u] [+FORMAT]", line: "Datum und Uhrzeit des Geräts ausgeben", text: "Die Uhr des Geräts läuft in der virtuellen Zeit der Simulation. FORMAT kennt %Y %m %d %H %M %S %F %T %s %a %b %j %Z %n %t und %%.", opts: &[("-u", "in UTC (die Geräte laufen ohnehin in UTC)")] },
    Page { name: "df", path: "/usr/bin/df", call: "df [-h] [PFAD]", line: "Größe und Belegung des Speichers zeigen", text: "Die Größe ist die des Gerätemodells; belegt ist, was seine Dateien halten.", opts: &[("-h", "in K, M, G statt in Blöcken zu 1024 Bytes")] },
    Page { name: "dhclient", path: "/usr/sbin/dhclient", call: "dhclient [-v] [-r] [IF]", line: "eine Adresse per DHCP holen oder zurückgeben", text: "", opts: &[] },
    Page { name: "dig", path: "/usr/bin/dig", call: "dig [@SERVER] NAME [TYP]", line: "den Namensserver fragen", text: "", opts: &[] },
    Page { name: "dirname", path: "/usr/bin/dirname", call: "dirname PFAD", line: "das Verzeichnis eines Pfads ausgeben", text: "Alles vor dem letzten /; . für einen Namen ohne /.", opts: &[] },
    Page { name: "du", path: "/usr/bin/du", call: "du [-s] [-h] [-a] [PFAD ...]", line: "den Platz von Verzeichnissen zeigen", text: "In Blöcken zu 1024 Bytes; jede Datei belegt ganze 4-KiB-Blöcke, jedes Verzeichnis einen.", opts: &[("-s", "nur die Summe je PFAD"), ("-h", "in K, M, G"), ("-a", "auch die Dateien")] },
    Page { name: "echo", path: B, call: "echo [-n] [-e] [TEXT ...]", line: "Text ausgeben", text: "", opts: &[("-n", "ohne Zeilenende"), ("-e", "\\n, \\t, \\\\ und \\xHH umsetzen")] },
    Page { name: "env", path: "/usr/bin/env", call: "env", line: "die exportierten Variablen zeigen", text: "", opts: &[] },
    Page { name: "exit", path: B, call: "exit", line: "eine weitere Sitzung beenden (die Konsole bleibt offen)", text: "Ein weiteres Terminal oder eine Telnet-Sitzung endet; die Konsole eines simulierten Geräts schließt sich nicht.", opts: &[] },
    Page { name: "export", path: B, call: "export [NAME[=WERT] ...]", line: "Variablen in die Umgebung übernehmen", text: "Ohne Argument zeigt export die Umgebung. Programme wie jfs sehen die exportierten Variablen.", opts: &[] },
    Page { name: "false", path: B, call: "false", line: "mit Status 1 enden", text: "", opts: &[] },
    Page { name: "fg", path: B, call: "fg [%N]", line: "auf einen Hintergrund-Job warten", text: "Die Konsole wartet, bis der Job endet; Strg+C beendet ihn.", opts: &[] },
    Page { name: "file", path: "/usr/bin/file", call: "file DATEI ...", line: "die Art von Dateien zeigen", text: "", opts: &[] },
    Page { name: "find", path: "/usr/bin/find", call: "find [PFAD ...] [-name MUSTER] [-iname MUSTER] [-type f|d] [-maxdepth N] [-mindepth N]", line: "Dateien in Verzeichnisbäumen suchen", text: "Ohne PFAD sucht find ab dem Arbeitsverzeichnis (.); die Treffer kommen in der Reihenfolge der Namen. MUSTER ist ein Platzhalter wie *.jfs (in Anführungszeichen, damit die Shell ihn nicht vorher ersetzt).", opts: &[("-name MUSTER", "der Name passt zum Muster"), ("-iname MUSTER", "ebenso, ohne auf Groß- und Kleinschreibung zu achten"), ("-type f|d", "nur Dateien oder nur Verzeichnisse"), ("-maxdepth N", "höchstens N Ebenen tief"), ("-mindepth N", "erst ab N Ebenen")] },
    Page { name: "grep", path: "/usr/bin/grep", call: "grep [-i] [-v] [-n] [-c] [-l] [-r] [-E] [-F] [-w] [-x] [-o] [-q] [-H] [-h] MUSTER [DATEI ...]", line: "Zeilen ausgeben, die zu einem Muster passen", text: "MUSTER ist ein regulärer Ausdruck nach POSIX (ohne Rückverweise): . [abc] [^a-z] [[:digit:]] ^ $ * und \\( \\) \\| \\{m,n\\} \\+ \\?, mit -E dieselben ohne \\. Dazu \\w \\s \\< \\> \\b. Ohne DATEI liest grep die Pipe. Status: 0 mit Treffer, 1 ohne, 2 bei einem Fehler.", opts: &[("-i", "Groß- und Kleinschreibung gleich behandeln"), ("-v", "die Zeilen, die nicht passen"), ("-n", "mit Zeilennummer"), ("-c", "nur die Zahl der Treffer"), ("-l", "nur die Namen der Dateien mit Treffern"), ("-r", "Verzeichnisse mit allem darin durchsuchen"), ("-E", "erweiterte Ausdrücke"), ("-F", "MUSTER ist Text, kein Ausdruck"), ("-w", "nur ganze Wörter"), ("-x", "nur ganze Zeilen"), ("-o", "nur die passenden Stücke"), ("-q", "nichts ausgeben, nur der Status"), ("-H / -h", "Dateinamen davor / nie")] },
    Page { name: "head", path: "/usr/bin/head", call: "head [-n N | -N] [-c N] [DATEI ...]", line: "den Anfang ausgeben", text: "Ohne -n die ersten 10 Zeilen.", opts: &[("-n N", "die ersten N Zeilen (-n -N: alle bis auf die letzten N)"), ("-c N", "die ersten N Bytes")] },
    Page { name: "help", path: B, call: "help [BEFEHL]", line: "die Befehle dieser Shell zeigen", text: "", opts: &[] },
    Page { name: "history", path: B, call: "history [N] | history -c", line: "die eingegebenen Zeilen zeigen", text: "!! wiederholt die letzte Zeile, !N die Zeile N, !-N die N-te von hinten.", opts: &[("-c", "die Liste leeren")] },
    Page { name: "host", path: "/usr/bin/host", call: "host NAME [SERVER]", line: "einen Namen nachschlagen", text: "", opts: &[] },
    Page { name: "hostname", path: "/usr/bin/hostname", call: "hostname [NAME]", line: "den Namen des Geräts zeigen oder ändern", text: "", opts: &[] },
    Page { name: "ip", path: "/usr/sbin/ip", call: "ip [-4|-6] [-br] address|link|route|neigh ...", line: "Adressen, Ports, Routen und Nachbarn", text: "ip OBJEKT help zeigt die Befehle eines Objekts.", opts: &[] },
    Page { name: "iw", path: "/usr/sbin/iw", call: "iw dev [IF info|link|scan|connect SSID [psk PASS]|disconnect|station dump]", line: "WLAN einstellen", text: "", opts: &[] },
    Page { name: "wpa_cli", path: "/usr/sbin/wpa_cli", call: "wpa_cli [-i IF] status|list_networks|reconfigure|logon|logoff|reauthenticate", line: "Den Supplicant fragen und steuern", text: "status sagt je Schnittstelle, ob und wie sie angemeldet ist. reconfigure liest die Datei neu.", opts: &[] },
    Page { name: "wpa_supplicant", path: "/usr/sbin/wpa_supplicant", call: "wpa_supplicant [-B] -i IF -c DATEI [-D TREIBER]", line: "802.1X und WPA-Enterprise: das Gerät am Netz anmelden", text: "Die Datei hat network-Blöcke mit key_mgmt (IEEE8021X am Kabel, WPA-EAP im WLAN mit ssid), eap (MD5 oder PWD), identity und password. Mit /etc/wpa_supplicant/wpa_supplicant.conf läuft der Supplicant von selbst.", opts: &[] },
    Page { name: "jfs", path: "/usr/bin/jfs", call: "jfs DATEI.jfs [ARGUMENTE ...] | jfs --test DATEI.jfs ...", line: "ein Skript als Prozess ausführen", text: "Das Skript läuft wie ein Dienst des Geräts (net, http, timer, disk, log ins Journal), aber im Terminal: print schreibt dorthin, config.args hält die ARGUMENTE, config.file den Pfad. Es endet, wenn nichts mehr zu tun ist (Status 0), nach einem Fehler (1) oder mit Strg+C (130); mit & läuft es im Hintergrund. Mit --test laufen die Funktionen test_… der Dateien nacheinander, jede in einem frischen Programm und höchstens 10 Sekunden lang; ein Fehler oder ein assert, das nicht hält, lässt einen Test scheitern. Status 1, wenn einer scheiterte oder keiner gefunden wurde.", opts: &[] },
    Page { name: "jobs", path: B, call: "jobs [-l] [-p]", line: "die Hintergrund-Jobs zeigen", text: "", opts: &[("-l", "mit PID"), ("-p", "nur die PIDs")] },
    Page { name: "kill", path: B, call: "kill [-s SIGNAL | -SIGNAL] PID | %N ...", line: "einen Job beenden", text: "Signale: TERM (Vorgabe), KILL, INT, HUP. Dienste des Geräts beendet kill nicht; sie haben ihre eigenen Befehle.", opts: &[("-l", "die Signale")] },
    Page { name: "less", path: "/usr/bin/less", call: "less [DATEI ...]", line: "Text seitenweise zeigen", text: "Leertaste: die nächste Seite, Enter: eine Zeile, q: aufhören.", opts: &[] },
    Page { name: "ls", path: "/usr/bin/ls", call: "ls [-l] [-a] [PFAD ...]", line: "Verzeichnisse auflisten", text: "", opts: &[("-l", "lang: Rechte, Größe, Zeit"), ("-a", "auch . und ..")] },
    Page { name: "mail", path: "/usr/bin/mail", call: "mail [-s BETREFF] ADRESSE ... | mail -H", line: "eine Mail verschicken", text: "Der Text kommt aus einer Pipe: echo TEXT | mail -s BETREFF ADRESSE.", opts: &[] },
    Page { name: "man", path: "/usr/bin/man", call: "man BEFEHL", line: "die Seite eines Befehls zeigen", text: "man sh beschreibt die Shell selbst.", opts: &[] },
    Page { name: "mkdir", path: "/usr/bin/mkdir", call: "mkdir [-p] VERZEICHNIS ...", line: "Verzeichnisse anlegen", text: "", opts: &[("-p", "auch fehlende darüber, kein Fehler, wenn es das Verzeichnis gibt")] },
    Page { name: "more", path: "/usr/bin/more", call: "more [DATEI ...]", line: "Text seitenweise zeigen", text: "Leertaste: die nächste Seite, Enter: eine Zeile, q: aufhören.", opts: &[] },
    Page { name: "mv", path: "/usr/bin/mv", call: "mv [-f] [-n] QUELLE ... ZIEL", line: "Dateien verschieben oder umbenennen", text: "Ist ZIEL ein Verzeichnis, kommen die Quellen dort hinein; Verzeichnisse ziehen mit allem darin um.", opts: &[("-n", "nichts überschreiben"), ("-f", "ohne zu fragen (mv fragt ohnehin nicht)")] },
    Page { name: "nc", path: "/usr/bin/nc", call: "nc [OPTIONEN] HOST PORT | nc -l PORT", line: "TCP und UDP von Hand", text: "nc -h zeigt die Optionen.", opts: &[] },
    Page { name: "nslookup", path: "/usr/bin/nslookup", call: "nslookup NAME [SERVER]", line: "einen Namen nachschlagen", text: "", opts: &[] },
    Page { name: "ping", path: "/usr/bin/ping", call: "ping [-c N] [-i S] [-s N] [-t TTL] [-W S] [-4|-6] ZIEL", line: "Echos schicken und die Antworten zeigen", text: "Ohne -c bis Strg+C. Status: 0 mit Antwort, 1 ohne, 2 wenn der Name unbekannt ist.", opts: &[] },
    Page { name: "printf", path: B, call: "printf FORMAT [ARGUMENT ...]", line: "formatiert ausgeben", text: "FORMAT kennt %s %d %x %c %b %% und \\n \\t.", opts: &[] },
    Page { name: "ps", path: "/usr/bin/ps", call: "ps [aux | -ef]", line: "die Prozesse des Geräts zeigen", text: "init, die Shell, die laufenden Dienste (PID 100 + ihre Nummer, wie bei top und ss -p) und die Hintergrund-Jobs, mit ihrer Rechenzeit.", opts: &[] },
    Page { name: "pwd", path: B, call: "pwd", line: "das Arbeitsverzeichnis ausgeben", text: "", opts: &[] },
    Page { name: "rm", path: "/usr/bin/rm", call: "rm [-f] [-r] DATEI ...", line: "Dateien löschen", text: "", opts: &[("-f", "fehlende Dateien sind kein Fehler"), ("-r", "Verzeichnisse mit allem darin")] },
    Page { name: "rmdir", path: "/usr/bin/rmdir", call: "rmdir VERZEICHNIS ...", line: "leere Verzeichnisse löschen", text: "", opts: &[] },
    Page { name: "script", path: "/usr/bin/script", call: "script [list|status|start|stop|restart|errors|output] [NAME]", line: "die Skripte des Geräts als Dienste", text: "script -h zeigt die Befehle.", opts: &[] },
    Page { name: "seq", path: "/usr/bin/seq", call: "seq [ERSTE [SCHRITT]] LETZTE", line: "Zahlen der Reihe nach ausgeben", text: "", opts: &[("-s TEXT", "TEXT statt des Zeilenendes zwischen den Zahlen"), ("-w", "mit Nullen auf gleiche Breite")] },
    Page { name: "sh", path: "/bin/sh", call: "BEFEHL [ARGUMENTE] [UMLEITUNGEN] [| BEFEHL ...] [&& | || | ; | & ...]", line: "die Shell dieses Geräts", text: "Pipes | geben die Ausgabe des linken Befehls an den rechten weiter, wenn der linke fertig ist (gesammelt, nicht Zeile für Zeile). Umleitungen: > DATEI, >> DATEI, < DATEI, 2> DATEI, 2>&1, &> DATEI und /dev/null; die Datei wird geschrieben, wenn der Befehl endet. Listen: ; nacheinander, && nur nach Erfolg, || nur nach einem Fehler, & im Hintergrund. Anführungszeichen: '...' wörtlich, \"...\" mit $VAR, \\ vor einem Zeichen. Platzhalter * ? [...] passen auf die Dateien des Geräts (sortiert; ohne Treffer bleibt das Wort). Variablen: NAME=WERT, $NAME, ${NAME}, $? (Status), $$, $!, ~ für $HOME. # beginnt einen Kommentar. Strg+C beendet den laufenden Befehl, $? ist dann 130.", opts: &[] },
    Page { name: "sleep", path: "/usr/bin/sleep", call: "sleep ZAHL[s|m|h|d] ...", line: "warten", text: "Mehrere Zeiten zählen zusammen; die Zeit ist die virtuelle der Simulation.", opts: &[] },
    Page { name: "sort", path: "/usr/bin/sort", call: "sort [-r] [-n] [-u] [-f] [-k N] [-t ZEICHEN] [DATEI ...]", line: "Zeilen sortieren", text: "Nach Bytes (wie mit LC_ALL=C).", opts: &[("-r", "absteigend"), ("-n", "nach der Zahl am Anfang"), ("-u", "gleiche Zeilen nur einmal"), ("-f", "Groß- und Kleinschreibung gleich"), ("-k N", "ab Feld N"), ("-t ZEICHEN", "Felder trennt dieses Zeichen")] },
    Page { name: "ss", path: "/usr/bin/ss", call: "ss [OPTIONEN] [FILTER]", line: "die Sockets des Geräts", text: "ss -h zeigt die Optionen.", opts: &[] },
    Page { name: "stat", path: "/usr/bin/stat", call: "stat DATEI ...", line: "die Angaben einer Datei zeigen", text: "", opts: &[] },
    Page { name: "ufw", path: "/usr/sbin/ufw", call: "ufw enable | disable | reset | status | default allow|deny|reject [incoming|outgoing] | allow|deny|reject [out] [from NETZ [to any port]] PORT[/tcp|/udp] | delete REGEL", line: "die kleine Firewall dieses Geräts", text: "Eingehend gilt ohne Regel deny, ausgehend allow. Was eine Regel oder die Vorgabe erlaubt, beginnt eine Sitzung, und die Antworten kommen zurück. Ping, ICMP-Fehler und die Antworten an einen DHCP-Client passieren immer.", opts: &[] },
    Page { name: "sysctl", path: "/usr/sbin/sysctl", call: "sysctl [-w] SCHLÜSSEL[=WERT]", line: "Einstellungen des Kernels", text: "", opts: &[] },
    Page { name: "tail", path: "/usr/bin/tail", call: "tail [-n N | -N | -n +N] [-c N] [DATEI ...]", line: "das Ende ausgeben", text: "Ohne -n die letzten 10 Zeilen.", opts: &[("-n N", "die letzten N Zeilen (+N: ab Zeile N)"), ("-c N", "die letzten N Bytes")] },
    Page { name: "tee", path: "/usr/bin/tee", call: "tee [-a] DATEI ...", line: "die Eingabe ausgeben und in Dateien schreiben", text: "", opts: &[("-a", "anhängen statt ersetzen")] },
    Page { name: "telnet", path: "/usr/bin/telnet", call: "telnet HOST [PORT]", line: "eine Telnet-Sitzung", text: "", opts: &[] },
    Page { name: "test", path: B, call: "test AUSDRUCK | [ AUSDRUCK ]", line: "Bedingungen prüfen", text: "Status 0, wenn der Ausdruck stimmt, sonst 1. Dateien: -e -f -d -s -r -w -x; Text: -z -n = !=; Zahlen: -eq -ne -lt -le -gt -ge; ! verneint, -a und -o verknüpfen.", opts: &[] },
    Page { name: "tftp", path: "/usr/bin/tftp", call: "tftp -g|-p [-l LOKAL] [-r ENTFERNT] HOST [PORT]", line: "eine Datei per TFTP holen oder schicken", text: "", opts: &[] },
    Page { name: "top", path: "/usr/bin/top", call: "top [-b] [-n N] [-d S]", line: "die Dienste und ihre Rechenzeit", text: "", opts: &[] },
    Page { name: "touch", path: "/usr/bin/touch", call: "touch DATEI ...", line: "Dateien anlegen oder ihre Zeit setzen", text: "", opts: &[] },
    Page { name: "traceroute", path: "/usr/bin/traceroute", call: "traceroute [-n] [-m MAX] [-q N] [-w S] ZIEL", line: "den Weg zu einem Ziel zeigen", text: "", opts: &[] },
    Page { name: "tree", path: "/usr/bin/tree", call: "tree [-a] [-d] [-L N] [VERZEICHNIS ...]", line: "Verzeichnisse als Baum zeigen", text: "", opts: &[("-a", "auch Dateien mit . am Anfang"), ("-d", "nur Verzeichnisse"), ("-L N", "höchstens N Ebenen")] },
    Page { name: "true", path: B, call: "true", line: "mit Status 0 enden", text: "", opts: &[] },
    Page { name: "type", path: B, call: "type BEFEHL ...", line: "zeigen, was ein Befehl ist", text: "", opts: &[] },
    Page { name: "uniq", path: "/usr/bin/uniq", call: "uniq [-c] [-d] [-u] [-i] [DATEI]", line: "gleiche Zeilen hintereinander zusammenfassen", text: "Nur direkt aufeinander folgende gleiche Zeilen; vorher sort, wenn alle gleichen zusammen sollen.", opts: &[("-c", "mit der Anzahl davor"), ("-d", "nur die mehrfachen"), ("-u", "nur die einfachen"), ("-i", "Groß- und Kleinschreibung gleich")] },
    Page { name: "unset", path: B, call: "unset NAME ...", line: "Variablen löschen", text: "", opts: &[] },
    Page { name: "wait", path: B, call: "wait [%N | PID]", line: "auf Hintergrund-Jobs warten", text: "", opts: &[] },
    Page { name: "wc", path: "/usr/bin/wc", call: "wc [-l] [-w] [-c] [-m] [DATEI ...]", line: "Zeilen, Wörter und Bytes zählen", text: "", opts: &[("-l", "Zeilen"), ("-w", "Wörter"), ("-c", "Bytes"), ("-m", "Zeichen")] },
    Page { name: "which", path: "/usr/bin/which", call: "which BEFEHL ...", line: "zeigen, wo ein Programm liegt", text: "Befehle der Shell selbst (cd, export ...) sind keine Programme; type zeigt sie.", opts: &[] },
    Page { name: "xargs", path: "/usr/bin/xargs", call: "xargs [BEFEHL [ARGUMENTE ...]]", line: "Wörter der Eingabe als Argumente übergeben", text: "Führt BEFEHL (sonst echo) einmal mit allen Wörtern der Eingabe am Ende aus.", opts: &[] },
    Page { name: "[", path: B, call: "[ AUSDRUCK ]", line: "Bedingungen prüfen (wie test)", text: "", opts: &[] },
    Page { name: "ping6", path: "/usr/bin/ping6", call: "ping6 ZIEL", line: "Echos über IPv6", text: "", opts: &[] },
    Page { name: "traceroute6", path: "/usr/bin/traceroute6", call: "traceroute6 ZIEL", line: "den Weg über IPv6 zeigen", text: "", opts: &[] },
    Page { name: "snmpget", path: "/usr/bin/snmpget", call: "snmpget -v VERSION -c COMMUNITY AGENT OID ...", line: "Werte eines SNMP-Agenten lesen", text: "Dazu snmpgetnext, snmpwalk, snmpbulkwalk und snmpset. Version 3 nimmt statt der Community -u BENUTZER, -l STUFE, -a VERFAHREN, -A PASSWORT, -x AES und -X PASSWORT.", opts: &[] },
    Page { name: "dbctl", path: "/usr/bin/dbctl", call: "dbctl [status|tables|locks|explain ANWEISUNG] [-s DIENST]", line: "die Datenbank des Geräts ansehen", text: "status nennt Verbindungen, Puffer und Trefferquote, Log, Anweisungen, Fehler und die Antwortzeit; tables die Tabellen mit ihren Zeilen, geänderten Zeilen, Seiten, Baumtiefe und Indizes; locks die Zeilensperren, wer sie hält und wie viele warten; explain sagt, wie eine Anweisung liefe: über einen Index oder alle Seiten, wie viele Zeilen und Seiten, und warum. Anfragen gehen mit curl -d an http://ADRESSE:7100/query.", opts: &[("-s DIENST", "welche Datenbank des Geräts, wenn es mehrere gibt")] },
    Page { name: "lbctl", path: "/usr/bin/lbctl", call: "lbctl [status] [-s DIENST]", line: "den Loadbalancer des Labors zeigen", text: "Dazu kvctl, cachectl, mqctl und workerctl für die anderen Dienste des Labors.", opts: &[] },
    Page { name: "who", path: "/usr/bin/who", call: "who", line: "die Sitzungen des Geräts zeigen", text: "Je Sitzung das Terminal (tty1 die Konsole, pts/N weitere Terminals und Telnet), seit wann und bei Telnet woher.", opts: &[] },
    Page { name: "w", path: "/usr/bin/w", call: "w", line: "die Sitzungen und was in ihnen läuft", text: "Die Uhrzeit, wie lange das Gerät läuft, je Sitzung woher, seit wann, wie lange nichts getippt wurde und der laufende Befehl.", opts: &[] },
    Page { name: "tty", path: "/usr/bin/tty", call: "tty", line: "das Terminal dieser Sitzung ausgeben", text: "", opts: &[] },
    Page { name: "systemctl", path: "/usr/bin/systemctl", call: "systemctl [OPTIONEN] start|stop|restart|status|enable|disable|is-active|is-failed|is-enabled|cat EINHEIT ... | systemctl list-units|list-unit-files|daemon-reload|reset-failed", line: "Dienste als Einheiten steuern", text: "Eine Einheit ist eine Datei /etc/systemd/system/NAME.service mit den Abschnitten [Unit], [Service] und [Install] (den Ordner legt mkdir -p an). ExecStart=jfs /PFAD/DATEI.jfs ARGUMENTE startet ein Skript, ExecStart=jfn-service ART [/PFAD/EINSTELLUNGEN.json] einen fertigen Dienst des Simulators. Restart=no|on-failure|always und RestartSec= sagen, was geschieht, wenn das Programm endet. enable legt eine kleine Datei in /etc/systemd/system/multi-user.target.wants an: die Einheit startet dann mit dem Gerät und das bleibt so, auch im gespeicherten Projekt. Die Dienste aus dem Projekt erscheinen als erzeugte Einheiten; starten und stoppen geht, enable und disable nicht. Nach einer Änderung an der Datei liest daemon-reload sie neu. Status: start, stop und restart 5 ohne eine solche Einheit, 1 wenn der Start scheitert; status 0 läuft, 3 läuft nicht, 4 gibt es nicht; is-active 0 oder 3; is-enabled und is-failed 0 oder 1.", opts: &[("--now", "enable und disable starten oder stoppen sie gleich mit"), ("-a", "list-units zeigt auch, was gerade nicht läuft"), ("--failed", "list-units zeigt nur, was gescheitert ist"), ("-q", "is-active und die anderen nur mit dem Status antworten"), ("-n N", "status zeigt N Zeilen des Logs")] },
    Page { name: "journalctl", path: "/usr/bin/journalctl", call: "journalctl [-u EINHEIT] [-n N] [-f] [-r]", line: "das Log des Geräts lesen", text: "Ohne Option alle Zeilen, die das Gerät noch hält (höchstens 1000), mit -u nur die einer Einheit: was ihr Dienst schrieb, bei Skripten auch ihr print, und was beim Starten und Enden geschah. Strg+C beendet -f.", opts: &[("-u EINHEIT", "nur diese Einheit"), ("-n N", "nur die letzten N Zeilen"), ("-f", "dabei bleiben und neue Zeilen zeigen, bis Strg+C"), ("-r", "die neuesten zuerst")] },
    Page { name: "ssh", path: "/usr/bin/ssh", call: "ssh [OPTIONEN] [BENUTZER@]HOST [BEFEHL ...]", line: "eine verschlüsselte Sitzung auf einem anderen Gerät", text: "Ohne BEFEHL eine Shell (Strg+C wirkt dort, die Zeile ~. beendet die Verbindung), mit BEFEHL läuft nur er; der Status ist seiner. Einen neuen Host-Schlüssel merkt sich ssh nach einer Frage in ~/.ssh/known_hosts, ein geänderter wird abgelehnt. Angemeldet wird mit ~/.ssh/id_ed25519 oder ~/.ssh/id_rsa, wenn es sie gibt, sonst mit einem Passwort.", opts: &[("-p PORT", "der Port (22)"), ("-l BENUTZER", "als dieser Benutzer"), ("-i DATEI", "dieser Schlüssel"), ("-c CHIFFRE", "chacha20-poly1305@openssh.com, aes128-ctr oder aes256-ctr"), ("-o StrictHostKeyChecking=yes|no|accept-new|ask", "was mit einem neuen Host-Schlüssel geschieht"), ("-o BatchMode=yes", "keine Fragen"), ("-o UserKnownHostsFile=DATEI", "eine andere Liste bekannter Hosts"), ("-q", "nur die Ausgabe der Gegenseite")] },
    Page { name: "ssh-keygen", path: "/usr/bin/ssh-keygen", call: "ssh-keygen [-t ed25519|rsa] [-b BITS] [-f DATEI] [-N \"\"] [-C KOMMENTAR] [-q]", line: "Schlüsselpaare für ssh erzeugen und ansehen", text: "Schreibt ~/.ssh/id_ed25519 und ~/.ssh/id_ed25519.pub (oder DATEI und DATEI.pub); der öffentliche Teil gehört auf dem Server nach ~/.ssh/authorized_keys. Dateien mit Passwort macht ssh-keygen hier nicht.", opts: &[("-t ART", "ed25519 oder rsa"), ("-b BITS", "die Länge eines RSA-Schlüssels"), ("-l -f DATEI", "Länge und Fingerabdruck"), ("-y -f DATEI", "der öffentliche Teil eines geheimen Schlüssels"), ("-R HOST", "einen Host aus ~/.ssh/known_hosts streichen")] },
    Page { name: "scp", path: "/usr/bin/scp", call: "scp [-P PORT] [-i DATEI] [-q] QUELLE ZIEL", line: "eine Datei über SSH kopieren", text: "Eine Seite heißt [BENUTZER@]HOST:PFAD; ein relativer Pfad gilt ab dem Heimatverzeichnis des Benutzers.", opts: &[("-P PORT", "der Port (22)"), ("-i DATEI", "dieser Schlüssel"), ("-q", "ohne die Zeile am Ende")] },
];

/// the page of a command (and its aliases)
pub fn page_of(name: &str) -> Option<&'static Page> {
    let n = match name {
        "jfn-sh" | "bash" => "sh",
        "snmpgetnext" | "snmpwalk" | "snmpbulkwalk" | "snmpset" => "snmpget",
        "kvctl" | "cachectl" | "mqctl" | "workerctl" => "lbctl",
        "netcat" => "nc",
        "logout" => "exit",
        x => x,
    };
    PAGES.iter().find(|p| p.name == n)
}

/// `text` wrapped at 72 columns with `indent` spaces before each line
fn wrap(text: &str, indent: usize, out: &mut String) {
    let pad: String = core::iter::repeat_n(' ', indent).collect();
    let mut line = String::new();
    for w in text.split(' ') {
        if !line.is_empty() && line.chars().count() + 1 + w.chars().count() + indent > 72 {
            out.push_str(&pad);
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(w);
    }
    if !line.is_empty() {
        out.push_str(&pad);
        out.push_str(&line);
        out.push('\n');
    }
}

/// the page as man shows it
pub fn man(name: &str) -> Option<String> {
    let p = page_of(name)?;
    let mut s = format!("NAME\n       {} - {}\n\nAUFRUF\n", p.name, p.line);
    wrap(p.call, 7, &mut s);
    if !p.text.is_empty() {
        s.push_str("\nBESCHREIBUNG\n");
        wrap(p.text, 7, &mut s);
    }
    if !p.opts.is_empty() {
        s.push_str("\nOPTIONEN\n");
        for (o, t) in p.opts {
            if o.chars().count() < 7 {
                let mut l = format!("       {:<7}", o);
                l.push_str(t);
                s.push_str(&l);
                s.push('\n');
            } else {
                s.push_str(&format!("       {}\n", o));
                wrap(t, 14, &mut s);
            }
        }
    }
    s.push_str(&format!("\nWO\n       {}\n", if p.path.is_empty() { "ein Befehl der Shell selbst" } else { p.path }));
    Some(s)
}

/// "Aufruf: ..." with the options (CMD --help)
pub fn usage(name: &str) -> String {
    let Some(p) = page_of(name) else { return String::new() };
    let mut s = format!("Aufruf: {}\n{}.\n", p.call, capital(p.line));
    for (o, t) in p.opts {
        s.push_str(&format!("  {:<12} {}\n", o, t));
    }
    s
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// the commands with a line each (help)
pub fn help() -> String {
    let mut v: Vec<&Page> = PAGES.iter().filter(|p| p.name != "[").collect();
    v.sort_by_key(|p| p.name);
    let mut s = String::from("Die Befehle dieser Shell (man BEFEHL zeigt mehr, man sh die Shell selbst: Pipes, Umleitungen, Listen, Platzhalter, Variablen, Jobs):\n");
    for p in v {
        s.push_str(&format!("  {:<12} {}\n", p.name, p.line));
    }
    s
}
