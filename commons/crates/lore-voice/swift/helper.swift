// lore-voice-helper: the macOS side of lore-voice. JSON lines: commands on stdin, events on stdout.
// Built by lore-voice's build.rs with swiftc (the Rust side spawns it and owns the protocol).
//
// Commands                                              Events
// {"cmd":"voices","req":1}                              {"ev":"voices","req":1,"voices":[{id,name,language,quality}]}
// {"cmd":"personal_voice","req":5,"ask":false}          {"ev":"personal_voice","req":5,"status":"authorized|denied|not_determined|unsupported"} (ask: the system's prompt when not determined)
// {"cmd":"permissions","req":2}                         {"ev":"permissions","req":2,"microphone":"granted|denied|undetermined","speech":…}
// {"cmd":"speak","id":5,"text","voice","rate","pitch","language"} {"ev":"speak_start","id":5} {"ev":"word","id":5,"start","len"} {"ev":"speak_end","id":5,"cancelled":false}
//   (word: the UTF-16 range of `text` about to be spoken)
// {"cmd":"stop_speaking"}
// {"cmd":"warm","voice","language"}                    (loads the voice silently; no answer)
// {"cmd":"listen","language":"zh-TW","continuous":false} {"ev":"listen_start","engine"} {"ev":"partial","text"} {"ev":"final","text"} {"ev":"listen_end"}
// {"cmd":"stop_listening"}
// {"cmd":"synth_file","req":3,"text","voice","rate","pitch","language","path"}   {"ev":"file","req":3,"path","frames"}
// {"cmd":"transcribe_file","req":4,"path","language"}   {"ev":"transcript","req":4,"text","engine"}
// errors: {"ev":"error","kind":"microphone|speech|unavailable|failed","message","req"?}
//
// STT: SpeechAnalyzer + SpeechTranscriber (macOS 26+) when the language is supported and
// installed (a supported one that is not installed starts downloading and SFSpeechRecognizer is
// used meanwhile); else SFSpeechRecognizer, on-device when it can. TTS: AVSpeechSynthesizer.

import AVFoundation
import Foundation
import Speech

// MARK: output

let outLock = NSLock()
func emit(_ d: [String: Any]) {
    guard let data = try? JSONSerialization.data(withJSONObject: d) else { return }
    outLock.lock()
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
    outLock.unlock()
}

/// A task that was cancelled (stop, a new start): the end of listening, never shown as an error.
func isCancel(_ e: Error) -> Bool {
    if e is CancellationError { return true }
    let ns = e as NSError
    return ns.domain == "Swift.CancellationError" || ns.code == NSUserCancelledError
}

func fail(_ kind: String, _ message: String, req: Any? = nil) {
    var d: [String: Any] = ["ev": "error", "kind": kind, "message": message]
    if let r = req { d["req"] = r }
    emit(d)
}

// MARK: voices and speaking

func qualityName(_ q: AVSpeechSynthesisVoiceQuality) -> String {
    switch q {
    case .premium: return "premium"
    case .enhanced: return "enhanced"
    default: return "default"
    }
}

/// Chinese and English voices without the novelty ones, plus the author's Personal Voice (any
/// language; listed by the system only once this program is authorized to use it).
func listVoices() -> [[String: Any]] {
    AVSpeechSynthesisVoice.speechVoices()
        .filter { isPersonal($0) || $0.language.hasPrefix("zh") || $0.language.hasPrefix("en") }
        .filter { !$0.voiceTraits.contains(.isNoveltyVoice) }
        .map { ["id": $0.identifier, "name": $0.name, "language": $0.language, "quality": qualityName($0.quality), "personal": isPersonal($0)] }
}

func isPersonal(_ v: AVSpeechSynthesisVoice) -> Bool {
    if #available(macOS 14, *) { return v.voiceTraits.contains(.isPersonalVoice) }
    return false
}

/// Personal Voice authorization: the current state, or (ask) the system's prompt first. The
/// prompt is attributed to the app this helper runs in (its responsible process).
func personalVoice(req: Any, ask: Bool) {
    guard #available(macOS 14, *) else {
        emit(["ev": "personal_voice", "req": req, "status": "unsupported"]); return
    }
    func name(_ st: AVSpeechSynthesizer.PersonalVoiceAuthorizationStatus) -> String {
        switch st {
        case .authorized: return "authorized"
        case .denied: return "denied"
        case .unsupported: return "unsupported"
        default: return "not_determined"
        }
    }
    let now = AVSpeechSynthesizer.personalVoiceAuthorizationStatus
    if !ask || now != .notDetermined {
        emit(["ev": "personal_voice", "req": req, "status": name(now)]); return
    }
    AVSpeechSynthesizer.requestPersonalVoiceAuthorization { st in
        DispatchQueue.main.async { emit(["ev": "personal_voice", "req": req, "status": name(st)]) }
    }
}

/// `rate` is a multiplier: 1 = the system's normal speed.
func utterance(_ text: String, voice: String?, rate: Double, language: String?, pitch: Double = 1) -> AVSpeechUtterance {
    let u = AVSpeechUtterance(string: text)
    if let v = voice, !v.isEmpty, let vv = AVSpeechSynthesisVoice(identifier: v) {
        u.voice = vv
    } else if let l = language, !l.isEmpty {
        u.voice = AVSpeechSynthesisVoice(language: l)
    }
    let r = Double(AVSpeechUtteranceDefaultSpeechRate) * (rate.isFinite && rate > 0 ? rate : 1)
    u.rate = Float(min(Double(AVSpeechUtteranceMaximumSpeechRate), max(Double(AVSpeechUtteranceMinimumSpeechRate), r)))
    u.pitchMultiplier = Float(min(2, max(0.5, pitch.isFinite ? pitch : 1)))
    return u
}

final class Speaker: NSObject, AVSpeechSynthesizerDelegate {
    let synth = AVSpeechSynthesizer()
    var ids: [ObjectIdentifier: Int] = [:]
    var fileSynth: AVSpeechSynthesizer?
    var warmSynth: AVSpeechSynthesizer?

    override init() {
        super.init()
        synth.delegate = self
    }

    func speak(id: Int, text: String, voice: String?, rate: Double, language: String?, pitch: Double) {
        let u = utterance(text, voice: voice, rate: rate, language: language, pitch: pitch)
        ids[ObjectIdentifier(u)] = id
        synth.speak(u)
    }

    func stop() { synth.stopSpeaking(at: .immediate) }

    private func id(_ u: AVSpeechUtterance) -> Int { ids[ObjectIdentifier(u)] ?? -1 }

    func speechSynthesizer(_ s: AVSpeechSynthesizer, didStart u: AVSpeechUtterance) {
        emit(["ev": "speak_start", "id": id(u)])
    }

    func speechSynthesizer(_ s: AVSpeechSynthesizer, willSpeakRangeOfSpeechString r: NSRange, utterance u: AVSpeechUtterance) {
        emit(["ev": "word", "id": id(u), "start": r.location, "len": r.length])
    }

    func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish u: AVSpeechUtterance) {
        emit(["ev": "speak_end", "id": id(u), "cancelled": false])
        ids.removeValue(forKey: ObjectIdentifier(u))
    }

    func speechSynthesizer(_ s: AVSpeechSynthesizer, didCancel u: AVSpeechUtterance) {
        emit(["ev": "speak_end", "id": id(u), "cancelled": true])
        ids.removeValue(forKey: ObjectIdentifier(u))
    }

    /// Load `voice` now (into a throwaway buffer, no sound), so the first sentence spoken with
    /// it starts without the voice's loading time.
    func warm(voice: String?, language: String?) {
        let s = AVSpeechSynthesizer()
        warmSynth = s
        s.write(utterance("嗯。", voice: voice, rate: 1, language: language)) { buffer in
            if let pcm = buffer as? AVAudioPCMBuffer, pcm.frameLength == 0 {
                DispatchQueue.main.async { if self.warmSynth === s { self.warmSynth = nil } }
            }
        }
    }

    /// Synthesise to a file (tests on a real Mac, previews): no sound is played.
    func toFile(req: Any, text: String, voice: String?, rate: Double, language: String?, pitch: Double, path: String) {
        let s = AVSpeechSynthesizer()
        fileSynth = s
        var file: AVAudioFile?
        var frames: AVAudioFrameCount = 0
        var done = false
        s.write(utterance(text, voice: voice, rate: rate, language: language, pitch: pitch)) { buffer in
            guard let pcm = buffer as? AVAudioPCMBuffer else { return }
            if pcm.frameLength == 0 {
                if !done {
                    done = true
                    file = nil
                    emit(["ev": "file", "req": req, "path": path, "frames": Int(frames)])
                }
                return
            }
            do {
                if file == nil {
                    file = try AVAudioFile(forWriting: URL(fileURLWithPath: path), settings: pcm.format.settings, commonFormat: pcm.format.commonFormat, interleaved: pcm.format.isInterleaved)
                }
                try file?.write(from: pcm)
                frames += pcm.frameLength
            } catch {
                if !done { done = true; fail("failed", "\(error.localizedDescription)", req: req) }
            }
        }
    }
}

// MARK: permissions

func micStatus() -> String {
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized: return "granted"
    case .notDetermined: return "undetermined"
    default: return "denied"
    }
}

func speechStatus() -> String {
    switch SFSpeechRecognizer.authorizationStatus() {
    case .authorized: return "granted"
    case .notDetermined: return "undetermined"
    default: return "denied"
    }
}

func withMicrophone(_ go: @escaping () -> Void) {
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized: go()
    case .notDetermined:
        AVCaptureDevice.requestAccess(for: .audio) { ok in
            DispatchQueue.main.async { ok ? go() : fail("microphone", "麥克風權限被拒絕") }
        }
    default: fail("microphone", "麥克風權限被拒絕")
    }
}

func withSpeechPermission(_ go: @escaping () -> Void) {
    switch SFSpeechRecognizer.authorizationStatus() {
    case .authorized: go()
    case .notDetermined:
        SFSpeechRecognizer.requestAuthorization { st in
            DispatchQueue.main.async { st == .authorized ? go() : fail("speech", "語音辨識權限被拒絕") }
        }
    default: fail("speech", "語音辨識權限被拒絕")
    }
}

// MARK: listening

/// One listening session. `continuous`: every pause ends a sentence (a `final` each) and it keeps
/// listening; otherwise (press and hold) the partials grow and `stop` sends one `final`.
final class Listener {
    var engine: AVAudioEngine?
    var continuous = false
    var finished = ""          // hold mode: the finalised part so far
    var running = false
    // SpeechAnalyzer
    var feed: (() -> Void)?   // ends the analyzer's input
    var analyzerStop: (() async -> Void)?
    var results: Task<Void, Never>?
    // SFSpeechRecognizer
    var sfRecognizer: SFSpeechRecognizer?
    var sfRequest: SFSpeechAudioBufferRecognitionRequest?
    var sfTask: SFSpeechRecognitionTask?
    var sfLast = ""
    var sfQuiet: DispatchWorkItem?
    var stopping = false

    func start(language: String, continuous: Bool) {
        stopNow()
        self.continuous = continuous
        finished = ""
        stopping = false
        withMicrophone { [self] in
            Task { @MainActor in await self.choose(language) }
        }
    }

    @MainActor func choose(_ language: String) async {
        if #available(macOS 26.0, *), SpeechTranscriber.isAvailable,
           let loc = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: language)) {
            let installed = await SpeechTranscriber.installedLocales.contains { $0.identifier(.bcp47) == loc.identifier(.bcp47) }
            if installed {
                do { try await startAnalyzer(loc); return } catch where isCancel(error) { return } catch { fail("failed", "SpeechTranscriber：\(error.localizedDescription)") }
            } else {
                download(loc)
            }
        }
        withSpeechPermission { [self] in self.startSF(language) }
    }

    @available(macOS 26.0, *)
    func download(_ loc: Locale) {
        Task {
            let t = SpeechTranscriber(locale: loc, preset: .progressiveTranscription)
            if let req = try? await AssetInventory.assetInstallationRequest(supporting: [t]) {
                emit(["ev": "notice", "message": "正在下載語音辨識模型"])
                try? await req.downloadAndInstall()
            }
        }
    }

    func micFormat() -> (AVAudioEngine, AVAudioFormat) {
        let e = AVAudioEngine()
        return (e, e.inputNode.outputFormat(forBus: 0))
    }

    @available(macOS 26.0, *)
    @MainActor func startAnalyzer(_ loc: Locale) async throws {
        let transcriber = SpeechTranscriber(locale: loc, transcriptionOptions: [], reportingOptions: [.volatileResults], attributeOptions: [])
        let analyzer = SpeechAnalyzer(modules: [transcriber])
        let (engine, inFmt) = micFormat()
        guard let fmt = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber], considering: inFmt) else {
            throw NSError(domain: "lore-voice", code: 1, userInfo: [NSLocalizedDescriptionKey: "沒有可用的音訊格式"])
        }
        let (stream, cont) = AsyncStream<AnalyzerInput>.makeStream()
        let converter = AVAudioConverter(from: inFmt, to: fmt)
        engine.inputNode.installTap(onBus: 0, bufferSize: 4096, format: inFmt) { buf, _ in
            if inFmt == fmt { cont.yield(AnalyzerInput(buffer: buf)); return }
            guard let converter else { return }
            let cap = AVAudioFrameCount(Double(buf.frameLength) * fmt.sampleRate / inFmt.sampleRate) + 16
            guard let out = AVAudioPCMBuffer(pcmFormat: fmt, frameCapacity: cap) else { return }
            var given = false
            var err: NSError?
            converter.convert(to: out, error: &err) { _, status in
                if given { status.pointee = .noDataNow; return nil }
                given = true
                status.pointee = .haveData
                return buf
            }
            if err == nil && out.frameLength > 0 { cont.yield(AnalyzerInput(buffer: out)) }
        }
        try await analyzer.prepareToAnalyze(in: fmt)
        engine.prepare()
        try engine.start()
        try await analyzer.start(inputSequence: stream)
        self.engine = engine
        self.feed = { cont.finish() }
        self.running = true
        self.analyzerStop = {
            try? await analyzer.finalizeAndFinishThroughEndOfInput()
        }
        results = Task { [self] in
            do {
                for try await r in transcriber.results {
                    let text = String(r.text.characters)
                    await MainActor.run { self.result(text, final: r.isFinal) }
                }
            } catch where isCancel(error) {
                // stopping (or a new start) ends the stream this way: not an error
            } catch {
                fail("failed", error.localizedDescription)
            }
        }
        emit(["ev": "listen_start", "engine": "speech-transcriber"])
    }

    @MainActor func result(_ text: String, final: Bool) {
        if continuous {
            let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if final { if !t.isEmpty { emit(["ev": "final", "text": t]) } } else { emit(["ev": "partial", "text": t]) }
        } else if final {
            finished += text
            emit(["ev": "partial", "text": finished.trimmingCharacters(in: .whitespacesAndNewlines)])
        } else {
            emit(["ev": "partial", "text": (finished + text).trimmingCharacters(in: .whitespacesAndNewlines)])
        }
    }

    func startSF(_ language: String) {
        guard let r = SFSpeechRecognizer(locale: Locale(identifier: language)), r.isAvailable else {
            fail("unavailable", "這台電腦不能辨識 \(language)")
            return
        }
        sfRecognizer = r
        let (engine, inFmt) = micFormat()
        engine.inputNode.installTap(onBus: 0, bufferSize: 4096, format: inFmt) { [weak self] buf, _ in
            self?.sfRequest?.append(buf)
        }
        newSFTask()
        engine.prepare()
        do { try engine.start() } catch { fail("failed", error.localizedDescription); return }
        self.engine = engine
        running = true
        emit(["ev": "listen_start", "engine": r.supportsOnDeviceRecognition ? "sf-speech-on-device" : "sf-speech"])
    }

    func newSFTask() {
        guard let r = sfRecognizer else { return }
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.shouldReportPartialResults = true
        if r.supportsOnDeviceRecognition { req.requiresOnDeviceRecognition = true }
        sfRequest = req
        sfLast = ""
        sfTask = r.recognitionTask(with: req) { [weak self] res, err in
            DispatchQueue.main.async { self?.sfResult(req, res, err) }
        }
    }

    func sfResult(_ req: SFSpeechAudioBufferRecognitionRequest, _ res: SFSpeechRecognitionResult?, _ err: Error?) {
        guard req === sfRequest else { return }
        let text = res?.bestTranscription.formattedString ?? sfLast
        if let res, !res.isFinal {
            sfLast = text
            emit(["ev": "partial", "text": (finished + text)])
            if continuous {
                // SFSpeechRecognizer does not end a sentence on a pause: 1.5 s without change does
                sfQuiet?.cancel()
                let w = DispatchWorkItem { [weak self] in self?.sfRequest?.endAudio() }
                sfQuiet = w
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.5, execute: w)
            }
            return
        }
        // final (or an error after some text): one sentence
        sfQuiet?.cancel()
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if continuous {
            if !t.isEmpty { emit(["ev": "final", "text": t]) }
            if running && !stopping { newSFTask() } else { end() }
        } else {
            finished += t
            end()
        }
        if err != nil && t.isEmpty && !stopping && !continuous { /* nothing heard */ }
    }

    /// Press-and-hold released / 常駐 turned off: the last words are finalised, then `listen_end`.
    func stop() {
        guard running, !stopping else { return }
        stopping = true
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        if let feed {
            feed()
            let s = analyzerStop
            Task { @MainActor in
                await s?()
                _ = await self.results?.value
                self.end()
            }
        } else if sfRequest != nil {
            sfQuiet?.cancel()
            sfRequest?.endAudio()
            // SFSpeechRecognizer may never call back after endAudio with no speech
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                if self?.running == true { self?.finished += self?.sfLast ?? ""; self?.end() }
            }
        } else {
            end()
        }
    }

    func end() {
        guard running else { return }
        running = false
        if !continuous {
            let t = finished.trimmingCharacters(in: .whitespacesAndNewlines)
            emit(["ev": "final", "text": t])
        }
        stopNow()
        emit(["ev": "listen_end"])
    }

    /// Drop everything without results.
    func stopNow() {
        running = false
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        engine = nil
        feed?(); feed = nil
        analyzerStop = nil
        results = nil
        sfQuiet?.cancel()
        sfTask?.cancel(); sfTask = nil
        sfRequest = nil
    }
}

// MARK: a file → text (checks on a real Mac without the microphone)

func transcribeFile(req: Any, path: String, language: String) {
    Task {
        if #available(macOS 26.0, *),
           let loc = await SpeechTranscriber.supportedLocale(equivalentTo: Locale(identifier: language)) {
            do {
                let t = SpeechTranscriber(locale: loc, preset: .transcription)
                let a = SpeechAnalyzer(modules: [t])
                let file = try AVAudioFile(forReading: URL(fileURLWithPath: path))
                let collect = Task { () -> String in
                    var s = ""
                    for try await r in t.results where r.isFinal { s += String(r.text.characters) }
                    return s
                }
                try await a.start(inputAudioFile: file, finishAfterFile: true)
                let text = try await collect.value
                emit(["ev": "transcript", "req": req, "text": text, "engine": "speech-transcriber"])
            } catch {
                fail("failed", error.localizedDescription, req: req)
            }
            return
        }
        fail("unavailable", "SpeechTranscriber 不支援 \(language)", req: req)
    }
}

// MARK: main

let speaker = Speaker()
let listener = Listener()

func handle(_ c: [String: Any]) {
    let req = c["req"] ?? NSNull()
    let str = { (k: String) in c[k] as? String }
    let rate = (c["rate"] as? Double) ?? 1
    let pitch = (c["pitch"] as? Double) ?? 1
    switch str("cmd") ?? "" {
    case "voices": emit(["ev": "voices", "req": req, "voices": listVoices()])
    case "personal_voice": personalVoice(req: req, ask: (c["ask"] as? Bool) ?? false)
    case "permissions": emit(["ev": "permissions", "req": req, "microphone": micStatus(), "speech": speechStatus()])
    case "speak":
        speaker.speak(id: (c["id"] as? Int) ?? -1, text: str("text") ?? "", voice: str("voice"), rate: rate, language: str("language"), pitch: pitch)
    case "stop_speaking": speaker.stop()
    case "warm": speaker.warm(voice: str("voice"), language: str("language"))
    case "listen": listener.start(language: str("language") ?? "zh-TW", continuous: (c["continuous"] as? Bool) ?? false)
    case "stop_listening": listener.stop()
    case "synth_file":
        speaker.toFile(req: req, text: str("text") ?? "", voice: str("voice"), rate: rate, language: str("language"), pitch: pitch, path: str("path") ?? "")
    case "transcribe_file": transcribeFile(req: req, path: str("path") ?? "", language: str("language") ?? "zh-TW")
    case let other: fail("failed", "unknown command \(other)", req: req)
    }
}

Thread {
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { continue }
        DispatchQueue.main.async { handle(obj) }
    }
    // the app went away
    DispatchQueue.main.async { listener.stopNow(); exit(0) }
}.start()

emit(["ev": "ready"])
dispatchMain()
