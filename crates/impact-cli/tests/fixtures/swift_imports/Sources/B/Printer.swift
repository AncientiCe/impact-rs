struct Printer {
    func prune(_ x: Int) -> Int {
        return x
    }

    func run(_ x: Int) -> Int {
        return self.prune(x)
    }
}
