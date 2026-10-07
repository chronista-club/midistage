import Foundation

/// CoreMIDI callback からも読める、アプリ自身の仮想ポートだけの許可表。
public final class NativeAccess: @unchecked Sendable {
    private let lock = NSLock()
    private var devices: [DeviceView] = []
    private var stamp: (epoch: String, session: String, sequence: UInt64)?
    public init() {}
    @discardableResult
    public func update(_ snapshot: Snapshot) -> Bool {
        lock.lock(); defer { lock.unlock() }
        if let stamp {
            guard stamp.epoch == snapshot.serverEpoch, stamp.session == snapshot.sessionID,
                  snapshot.sequence > stamp.sequence else { return false }
        }
        stamp = (snapshot.serverEpoch, snapshot.sessionID, snapshot.sequence)
        devices = snapshot.devices.filter(snapshot.owns)
        return true
    }
    public func revoke(profileID: String) {
        lock.lock(); defer { lock.unlock() }
        devices.removeAll { $0.profileID == profileID }
    }
    public func clear() {
        lock.lock(); defer { lock.unlock() }
        devices.removeAll()
        stamp = nil
    }
    public func allowsInput(_ name: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return devices.contains { $0.nativeInputs.contains(name) }
    }
    public func allowsOutput(_ name: String) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return devices.contains { $0.nativeOutputs.contains(name) }
    }
    @discardableResult
    public func withInput(_ name: String, _ body: () -> Void) -> Bool {
        lock.lock(); defer { lock.unlock() }
        guard devices.contains(where: { $0.nativeInputs.contains(name) }) else { return false }
        body()
        return true
    }
}
