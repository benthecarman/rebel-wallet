import UIKit

extension UIApplication {
    func rebelTopViewController() -> UIViewController? {
        connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first { $0.isKeyWindow }?
            .rootViewController?
            .rebelTopPresentedViewController()
    }
}

extension UIViewController {
    func rebelTopPresentedViewController() -> UIViewController {
        if let presentedViewController {
            return presentedViewController.rebelTopPresentedViewController()
        }
        if let navigationController = self as? UINavigationController {
            return navigationController.visibleViewController?.rebelTopPresentedViewController() ?? navigationController
        }
        if let tabBarController = self as? UITabBarController {
            return tabBarController.selectedViewController?.rebelTopPresentedViewController() ?? tabBarController
        }
        return self
    }
}
