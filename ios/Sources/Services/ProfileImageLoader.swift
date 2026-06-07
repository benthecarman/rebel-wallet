import Foundation
import SwiftUI
import UIKit

@MainActor
final class ProfileImageLoader: ObservableObject {
    @Published private(set) var image: UIImage?
    @Published private(set) var isLoading = false

    private static let cache = NSCache<NSURL, UIImage>()
    private static var inFlight: [URL: Task<UIImage?, Never>] = [:]

    private var url: URL?
    private var loadTask: Task<Void, Never>?

    func load(_ nextURL: URL) {
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
        if let cached = cache.object(forKey: url as NSURL) {
            return cached
        }

        if let existing = inFlight[url] {
            return await existing.value
        }

        let task = Task.detached(priority: .utility) { () -> UIImage? in
            var request = URLRequest(url: url)
            request.cachePolicy = .returnCacheDataElseLoad

            guard
                let (data, response) = try? await URLSession.shared.data(for: request),
                let httpResponse = response as? HTTPURLResponse,
                200..<300 ~= httpResponse.statusCode,
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

    deinit {
        loadTask?.cancel()
    }
}
