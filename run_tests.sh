#!/bin/bash
# Main test runner for ChatSafe
# Runs all test suites and provides summary

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
TESTS_DIR="./tests"
API_URL="http://127.0.0.1:8081/healthz"

# Check if server is running
check_server() {
    echo -n "Checking if server is running... "
    if curl -s $API_URL > /dev/null 2>&1; then
        echo -e "${GREEN}✓${NC}"
        return 0
    else
        echo -e "${RED}✗${NC}"
        echo -e "${RED}Error: Server not running on port 8081${NC}"
        echo "Please start the server with: ./target/release/chatsafe-server"
        exit 1
    fi
}

# Run a test suite
run_suite() {
    local suite_name="$1"
    local suite_path="$2"
    
    echo -e "\n${BLUE}Running $suite_name...${NC}"
    
    if [ -f "$suite_path" ]; then
        if bash "$suite_path"; then
            echo -e "${GREEN}✓ $suite_name passed${NC}"
            return 0
        else
            echo -e "${RED}✗ $suite_name failed${NC}"
            return 1
        fi
    else
        echo -e "${YELLOW}⚠ $suite_name not found${NC}"
        return 1
    fi
}

# Main execution
main() {
    echo "======================================"
    echo "     ChatSafe Test Suite Runner"
    echo "======================================"
    
    # Check prerequisites
    check_server
    
    local total_suites=0
    local passed_suites=0
    local failed_suites=0
    
    # Core functionality tests
    if run_suite "Golden Tests" "$TESTS_DIR/test_golden.sh"; then
        passed_suites=$((passed_suites + 1))
    else
        failed_suites=$((failed_suites + 1))
    fi
    total_suites=$((total_suites + 1))
    
    # Integration tests
    if run_suite "Comprehensive Tests" "$TESTS_DIR/test_comprehensive.sh"; then
        passed_suites=$((passed_suites + 1))
    else
        failed_suites=$((failed_suites + 1))
    fi
    total_suites=$((total_suites + 1))
    
    # Security tests
    if run_suite "Security Tests" "$TESTS_DIR/test_security.sh"; then
        passed_suites=$((passed_suites + 1))
    else
        failed_suites=$((failed_suites + 1))
    fi
    total_suites=$((total_suites + 1))
    
    # Role pollution tests
    if run_suite "Role Pollution Tests" "$TESTS_DIR/test_role_pollution.sh"; then
        passed_suites=$((passed_suites + 1))
    else
        failed_suites=$((failed_suites + 1))
    fi
    total_suites=$((total_suites + 1))
    
    # Optional/specialized tests
    echo -e "\n${BLUE}Optional Test Suites:${NC}"
    
    # These are informational, don't count toward pass/fail
    if [ -f "$TESTS_DIR/test_confusion.sh" ]; then
        echo "  - test_confusion.sh (edge cases)"
    fi
    if [ -f "$TESTS_DIR/test_dod.sh" ]; then
        echo "  - test_dod.sh (definition of done)"
    fi
    if [ -f "$TESTS_DIR/test_overload.sh" ]; then
        echo "  - test_overload.sh (load testing)"
    fi
    
    # Unit tests
    echo -e "\n${BLUE}Running Rust unit tests...${NC}"
    if cargo test --quiet; then
        echo -e "${GREEN}✓ Unit tests passed${NC}"
        passed_suites=$((passed_suites + 1))
    else
        echo -e "${RED}✗ Unit tests failed${NC}"
        failed_suites=$((failed_suites + 1))
    fi
    total_suites=$((total_suites + 1))
    
    # Summary
    echo -e "\n======================================"
    echo -e "Test Summary: ${passed_suites}/${total_suites} suites passed"
    
    if [ $failed_suites -eq 0 ]; then
        echo -e "${GREEN}All test suites passed!${NC}"
        echo -e "\nFor detailed coverage report, see: docs/test_coverage.md"
        exit 0
    else
        echo -e "${RED}${failed_suites} test suites failed${NC}"
        echo -e "\nFor test gaps and improvements needed, see: docs/test_coverage.md"
        exit 1
    fi
}

# Handle arguments
case "${1:-}" in
    --help|-h)
        echo "Usage: $0 [options]"
        echo ""
        echo "Options:"
        echo "  --help, -h     Show this help message"
        echo "  --security     Run only security tests"
        echo "  --quick        Run only quick tests (skip comprehensive)"
        echo "  --unit         Run only unit tests"
        echo ""
        echo "Individual test suites can be run directly:"
        echo "  ./tests/test_golden.sh"
        echo "  ./tests/test_comprehensive.sh"
        echo "  ./tests/test_security.sh"
        echo "  ./tests/test_role_pollution.sh"
        exit 0
        ;;
    --security)
        check_server
        run_suite "Security Tests" "$TESTS_DIR/test_security.sh"
        exit $?
        ;;
    --unit)
        cargo test
        exit $?
        ;;
    --quick)
        check_server
        run_suite "Golden Tests" "$TESTS_DIR/test_golden.sh"
        cargo test --quiet
        exit $?
        ;;
    *)
        main "$@"
        ;;
esac