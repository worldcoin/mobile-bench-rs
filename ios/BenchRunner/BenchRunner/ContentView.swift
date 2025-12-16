import SwiftUI

struct ContentView: View {
    @State private var report: String = "Running benchmark..."

    var body: some View {
        ScrollView {
            Text(report)
                .font(.system(.body, design: .monospaced))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
        }
        .background(Color(UIColor.systemBackground))
        .onAppear {
            Task {
                report = await BenchRunnerFFI.runCurrentBenchmark()
            }
        }
    }
}

#Preview
struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
