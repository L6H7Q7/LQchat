# Offline checks using the project's existing Kotlin 1.9.25 compiler cache. No installs or downloads.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$cache = 'D:\AI\path\Cache\Gradle\caches\modules-2\files-2.1'
$java = 'D:\AI\path\Java\jdk-17.0.20.1+1\bin\java.exe'
function Find-ReminderJar([string]$relative) {
    $found = @(Get-ChildItem -LiteralPath (Join-Path $cache $relative) -Recurse -Filter '*.jar')
    if ($found.Count -ne 1) { throw "Expected one cached JAR: $relative" }
    return $found[0].FullName
}
$stdlib = Find-ReminderJar 'org.jetbrains.kotlin\kotlin-stdlib\1.9.25'
$annotations = Find-ReminderJar 'org.jetbrains\annotations\13.0'
$compiler = @(
    (Find-ReminderJar 'org.jetbrains.kotlin\kotlin-compiler-embeddable\1.9.25'),
    $stdlib,
    (Find-ReminderJar 'org.jetbrains.kotlin\kotlin-reflect\1.6.10'),
    (Find-ReminderJar 'org.jetbrains.intellij.deps\trove4j\1.0.20200330'),
    $annotations
)
$output = Join-Path $repo 'artifacts\custom-reminders\schedule-check'
New-Item -ItemType Directory -Path $output -Force | Out-Null
$source = Join-Path $repo 'src-tauri\gen\android\app\src\main\java\com\lanchat\app\CustomReminderSchedule.kt'
& $java '-cp' ($compiler -join ';') 'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler' '-no-stdlib' '-no-reflect' '-jvm-target' '1.8' '-classpath' "$stdlib;$annotations" '-d' $output $source (Join-Path (Split-Path -Parent $source) 'CustomReminderForwarding.kt') (Join-Path $PSScriptRoot 'CustomReminderScheduleCheck.kt')
if ($LASTEXITCODE -ne 0) { throw 'Kotlin schedule check compilation failed' }
& $java '-cp' "$output;$stdlib" 'com.lanchat.app.CustomReminderScheduleCheckKt'
if ($LASTEXITCODE -ne 0) { throw 'Kotlin schedule checks failed' }
