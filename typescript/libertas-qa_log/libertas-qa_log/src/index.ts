/// <reference types="@libertas-ai/types" />
/// libertas-qa_log
/// Test libertasLog
/// @LibertasDefaultLocale("en")
import * as libertas from '@libertas-ai/consts';

export function test_log() {
    libertasLog(libertas.LogLevel.Info, "Hello world.")
    libertasLog(libertas.LogLevel.Info, "Hello world 6.")
}
