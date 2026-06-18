import Foundation

func truncateMiddle(_ value: String, maxLength: Int, prefixCount: Int? = nil) -> String {
    guard value.count > maxLength, maxLength > 3 else { return value }
    let leading = prefixCount ?? (maxLength - 3) / 2
    let trailing = maxLength - leading - 3
    guard leading > 0, trailing > 0 else { return value }
    return "\(value.prefix(leading))...\(value.suffix(trailing))"
}
