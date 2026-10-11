/// <reference types="@libertas-ai/types" />
/// libertas-qa_log
/// Test libertasLog
/// @LibertasDefaultLocale("en")
import * as libertas from '@libertas-ai/consts';

/** Reserved key schema; this sample does not persist records. */
export type LogDataKey = { kind: 0 } | { kind: 1 };
/** Reserved stored values. */
export type LogData = { kind: 0 } | { kind: 1 };

/** @LibertasDataSchema(key=LogDataKey,value=LogData) */
export function test_log() {
    libertasLog(libertas.LogLevel.Info, "Hello world.")
    libertasLog(libertas.LogLevel.Info, "Hello world 6.")
}
