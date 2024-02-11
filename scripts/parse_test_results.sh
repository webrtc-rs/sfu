#!/bin/bash

# Initialize accumulated totals
total_passed=0
total_failed=0
total_ignored=0
total_measured=0
total_filtered=0
total_time=0

# Arrays to store passed, failed, and ignored tests
passed_tests=()
failed_tests=()
ignored_tests=()

# Read each line from the log file
while IFS= read -r line; do
    # Check if the line indicates test results
    if [[ $line =~ test\ result:\ (FAILED|ok).\ ([0-9]+)\ passed\;\ ([0-9]+)\ failed\;\ ([0-9]+)\ ignored\;\ ([0-9]+)\ measured\;\ ([0-9]+)\ filtered\ out\;\ finished\ in\ ([0-9]+\.[0-9]+)s ]]; then
        # Extract the test results
        result="${BASH_REMATCH[1]}"
        passed="${BASH_REMATCH[2]}"
        failed="${BASH_REMATCH[3]}"
        ignored="${BASH_REMATCH[4]}"
        measured="${BASH_REMATCH[5]}"
        filtered="${BASH_REMATCH[6]}"
        time="${BASH_REMATCH[7]}"

        # Accumulate the test results
        total_passed=$((total_passed + passed))
        total_failed=$((total_failed + failed))
        total_ignored=$((total_ignored + ignored))
        total_measured=$((total_measured + measured))
        total_filtered=$((total_filtered + filtered))
        total_time=$(bc <<< "$total_time + $time")
    elif [[ $line =~ test\ (.*)\ \.\.\.\ (FAILED|ok|ignored) ]]; then
        # Extract the test name and result
        test_name="${BASH_REMATCH[1]}"
        test_result="${BASH_REMATCH[2]}"

        # Add the test name to the appropriate array based on the result
        case "$test_result" in
            "ok")
                passed_tests+=("$test_name")
                ;;
            "FAILED")
                failed_tests+=("$test_name")
                ;;
            "ignored")
                ignored_tests+=("$test_name")
                ;;
        esac
    fi
done < "$1"

# Print passed tests with indentation
echo "Total Passed: $total_passed"
for test_name in "${passed_tests[@]}"; do
    echo "  $test_name"
done

# Print failed tests with indentation
echo "Total Failed: $total_failed"
for test_name in "${failed_tests[@]}"; do
    echo "  $test_name"
done

# Print ignored tests with indentation
echo "Total Ignored: $total_ignored"
for test_name in "${ignored_tests[@]}"; do
    echo "  $test_name"
done

# Return non-zero exit code if there are failed tests
if [[ $total_failed -gt 0 ]]; then
    exit 1
else
    exit 0
fi
