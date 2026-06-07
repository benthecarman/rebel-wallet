import Foundation
import SwiftUI
import UIKit

@MainActor
final class ProfileImageLoader: ObservableObject {
    @Published private(set) var image: UIImage?
    @Published private(set) var isLoading = false

    private nonisolated static let maxImageBytes = 5_000_000
    private static let cache = NSCache<NSURL, UIImage>()
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

    func load(_ nextURL: URL) {
        guard Self.canLoad(nextURL) else {
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
            let loaded = await Self.image(for: nextURL)
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

    private static func image(for url: URL) async -> UIImage? {
        guard canLoad(url) else { return nil }

        if let cached = cache.object(forKey: url as NSURL) {
            return cached
        }

        if let existing = inFlight[url] {
            return await existing.value
        }

        let task = Task.detached(priority: .utility) { () -> UIImage? in
            var request = URLRequest(url: url)
            request.cachePolicy = .reloadIgnoringLocalCacheData

            guard
                let (data, response) = try? await session.data(for: request),
                let httpResponse = response as? HTTPURLResponse,
                200..<300 ~= httpResponse.statusCode,
                data.count <= maxImageBytes,
                let image = UIImage(data: data)
            else {
                return nil
            }

            return image
        }

        inFlight[url] = task
        let loaded = await task.value
        if let loaded {
            cache.setObject(loaded, forKey: url as NSURL)
        }
        inFlight[url] = nil
        return loaded
    }

    private static func canLoad(_ url: URL) -> Bool {
        guard let scheme = url.scheme?.lowercased() else { return false }
        return scheme == "https" || scheme == "http"
    }

    deinit {
        loadTask?.cancel()
    }
}
