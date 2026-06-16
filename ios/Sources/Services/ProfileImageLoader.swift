import Foundation
import SwiftUI
import UIKit

@MainActor
final class ProfileImageLoader: ObservableObject {
    @Published private(set) var image: UIImage?
    @Published private(set) var isLoading = false

    private nonisolated static let maxImageBytes = 5_000_000
    private static let cache: NSCache<NSURL, UIImage> = {
        let cache = NSCache<NSURL, UIImage>()
        cache.countLimit = 200
        cache.totalCostLimit = 50 * 1024 * 1024
        return cache
    }()
    private static var inFlight: [URL: Task<UIImage?, Never>] = [:]
    private static let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.urlCache = nil
        configuration.timeoutIntervalForRequest = 15
        configuration.timeoutIntervalForResource = 30
        return URLSession(configuration: configuration)
    }()

    private var url: URL?
    private var loadTask: Task<Void, Never>?

    /// Loads an avatar URL supplied by Rust state.
    ///
    /// Profile pictures are cache-managed by the Rust core. Normal app views
    /// should therefore load only `file://` URLs so they cannot bypass
    /// `profiles.sqlite3` and the normalized `profile_pictures` disk cache.
    /// The remote opt-in is reserved for explicit edit previews.
    func load(_ nextURL: URL, normalizer: FfiApp? = nil, allowRemote: Bool = false) {
        guard Self.canLoad(nextURL, allowRemote: allowRemote) else {
            reset()
            return
        }
        guard url != nextURL || image == nil else { return }

        loadTask?.cancel()
        url = nextURL

        if let cached = Self.cache.object(forKey: nextURL as NSURL) {
            image = cached
            isLoading = false
            return
        }

        image = nil
        isLoading = true
        loadTask = Task { [weak self] in
            let loaded = await Self.image(for: nextURL, normalizer: normalizer, allowRemote: allowRemote)
            guard !Task.isCancelled else { return }
            guard let self, self.url == nextURL else { return }
            self.image = loaded
            self.isLoading = false
        }
    }

    func reset() {
        loadTask?.cancel()
        loadTask = nil
        url = nil
        image = nil
        isLoading = false
    }

    private static func image(for url: URL, normalizer: FfiApp?, allowRemote: Bool) async -> UIImage? {
        guard canLoad(url, allowRemote: allowRemote) else { return nil }

        if let cached = cache.object(forKey: url as NSURL) {
            return cached
        }

        if let existing = inFlight[url] {
            return await existing.value
        }

        let task = Task.detached(priority: .utility) { () -> UIImage? in
            if url.isFileURL {
                let fileURL = URL(fileURLWithPath: url.path)
                guard
                    let data = try? Data(contentsOf: fileURL),
                    data.count <= maxImageBytes
                else {
                    return nil
                }
                return decodeImage(data, normalizer: normalizer)
            }

            var request = URLRequest(url: url)
            request.cachePolicy = .reloadIgnoringLocalCacheData

            guard
                let (data, response) = try? await session.data(for: request),
                let httpResponse = response as? HTTPURLResponse,
                200..<300 ~= httpResponse.statusCode,
                data.count <= maxImageBytes
            else {
                return nil
            }

            return decodeImage(data, normalizer: normalizer)
        }

        inFlight[url] = task
        let loaded = await task.value
        if let loaded {
            cache.setObject(loaded, forKey: url as NSURL, cost: imageCost(loaded))
        }
        inFlight[url] = nil
        return loaded
    }

    private nonisolated static func decodeImage(_ data: Data, normalizer: FfiApp?) -> UIImage? {
        if let image = UIImage(data: data) {
            return image
        }
        guard
            let normalized = normalizer?.normalizeProfileImageToJpeg(imageBytes: data)
        else {
            return nil
        }
        return UIImage(data: normalized)
    }

    private static func imageCost(_ image: UIImage) -> Int {
        guard let cgImage = image.cgImage else { return 0 }
        return cgImage.bytesPerRow * cgImage.height
    }

    private static func canLoad(_ url: URL, allowRemote: Bool) -> Bool {
        guard let scheme = url.scheme?.lowercased() else { return false }
        return scheme == "file" || (allowRemote && (scheme == "https" || scheme == "http"))
    }

    deinit {
        loadTask?.cancel()
    }
}
