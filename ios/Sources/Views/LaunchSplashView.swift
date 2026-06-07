import SwiftUI

struct LaunchSplashView: View {
    private let phrase = LaunchSplashPhrase.selected

    var body: some View {
        VStack(spacing: 18) {
            Spacer()
            RebelMark(size: 96)
            Text("Rebel Wallet")
                .font(.largeTitle.bold())
            RebelLoadingText(text: phrase)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(24)
        .background(pageBackground.ignoresSafeArea())
        .foregroundStyle(primaryText)
    }
}

struct RebelLoadingText: View {
    let text: String
    @State private var shimmer = false

    var body: some View {
        Text(text)
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(primaryText)
            .multilineTextAlignment(.center)
            .lineLimit(2)
            .overlay {
                GeometryReader { proxy in
                    LinearGradient(
                        colors: [
                            .clear,
                            rebelRed.opacity(0.35),
                            rebelRed,
                            rebelRed.opacity(0.35),
                            .clear
                        ],
                        startPoint: .leading,
                        endPoint: .trailing
                    )
                    .frame(width: max(proxy.size.width * 0.7, 120))
                    .offset(x: shimmer ? proxy.size.width : -proxy.size.width)
                    .blur(radius: 0.5)
                    .mask {
                        Text(text)
                            .font(.subheadline.weight(.semibold))
                            .multilineTextAlignment(.center)
                            .lineLimit(2)
                            .frame(width: proxy.size.width, height: proxy.size.height)
                    }
                }
                .clipped()
            }
            .animation(.easeInOut(duration: 1.45).repeatForever(autoreverses: false), value: shimmer)
            .onAppear {
                shimmer = true
            }
    }
}

private enum LaunchSplashPhrase {
    static let selected = phrases.randomElement() ?? phrases[0]

    private static let phrases = [
        "joining the rebellion",
        "welcome to the rebellion",
        "routing around the empire",
        "arming your wallet",
        "lighting the signal",
        "checking the resistance network",
        "preparing your rebel base"
    ]
}
