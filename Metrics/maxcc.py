import lizard
import sys

def analyze_file(filename):
    analysis = lizard.analyze_file(filename)
    functions = analysis.function_list

    if not functions:
        print(f"No functions found in {filename}")
        return

    print(f"\nCyclomatic Complexity analysis for {filename}\n")
    print(f"{'Function':40} {'CC':>4}")
    print("-" * 50)

    max_func = None
    max_cc = 0

    for func in functions:
        name = func.name
        cc = func.cyclomatic_complexity
        print(f"{name:40} {cc:>4}")
        if cc > max_cc:
            max_cc = cc
            max_func = name

    print("-" * 50)
    print(f"Most complex function: {max_func} (CC = {max_cc})")
    print(f"Maximum complexity: {max_cc}\n")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python cc_report.py <file.rs>")
        sys.exit(1)

    file_path = sys.argv[1]
    analyze_file(file_path)
