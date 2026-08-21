#!/usr/bin/env Rscript

suppressPackageStartupMessages(library(jsonlite))

payload <- fromJSON(file("stdin"), simplifyVector = FALSE)
columns <- unlist(payload$columns, use.names = FALSE)
row_values <- lapply(payload$rows, function(row) unlist(row, use.names = FALSE))
if (length(columns) < 2 || length(row_values) < 2 || !all(vapply(row_values, length, integer(1)) == length(columns))) {
  stop("R rows do not match declared columns")
}
data <- as.data.frame(do.call(rbind, row_values), stringsAsFactors = FALSE)
names(data) <- columns
data[] <- lapply(data, type.convert, as.is = TRUE)
method <- payload$method
outcome <- payload$outcome
predictors <- unlist(payload$predictors, use.names = FALSE)
set.seed(as.integer(payload$seed))

safe_name <- function(value) {
  is.character(value) && length(value) == 1 && grepl("^[A-Za-z_][A-Za-z0-9_]{0,127}$", value)
}

if (!safe_name(outcome) || length(predictors) < 1 || !all(vapply(predictors, safe_name, logical(1)))) {
  stop("unsafe or missing R column name")
}
if (!all(c(outcome, predictors) %in% names(data))) {
  stop("R input references an unavailable column")
}

if (method == "linear_regression") {
  formula <- reformulate(predictors, response = outcome)
  model <- lm(formula, data = data)
  summary_model <- summary(model)
  result <- list(
    method = method,
    estimates = as.list(coef(model)),
    diagnostics = list(
      r_squared = unname(summary_model$r.squared),
      adjusted_r_squared = unname(summary_model$adj.r.squared),
      sigma = unname(summary_model$sigma),
      rows = nrow(data)
    )
  )
} else if (method == "welch_t_test") {
  group <- predictors[[1]]
  test <- t.test(data[[outcome]] ~ as.factor(data[[group]]), var.equal = FALSE)
  result <- list(
    method = method,
    estimates = list(statistic = unname(test$statistic), estimate = as.list(test$estimate)),
    diagnostics = list(p_value = test$p.value, confidence_interval = as.list(test$conf.int))
  )
} else if (method == "chi_squared_test") {
  predictor <- predictors[[1]]
  test <- chisq.test(table(data[[outcome]], data[[predictor]]))
  result <- list(
    method = method,
    estimates = list(statistic = unname(test$statistic)),
    diagnostics = list(p_value = test$p.value, degrees_of_freedom = unname(test$parameter))
  )
} else {
  stop("unsupported governed R method")
}

cat(toJSON(result, auto_unbox = TRUE, digits = NA, null = "null"))
